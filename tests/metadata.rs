//! Tests for frame metadata parsing using custom serializers.
//!
//! They ensure parse callbacks run before deserialization and errors fall back correctly.
#![cfg(not(loom))]

use std::sync::{
    Arc,
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use wireframe::{
    app::{Envelope, Packet},
    frame::FrameMetadata,
    message::DeserializeContext,
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
};
use wireframe_testing::{TestResult, TestSerializer, drive_with_bincode};

#[path = "common/context_capturing_serializer.rs"]
mod context_capturing_serializer;

use context_capturing_serializer::{CapturedDeserializeContext, ContextCapturingSerializer};

type TestApp<S = BincodeSerializer> = wireframe::app::WireframeApp<S, (), Envelope>;

fn mock_wireframe_app_with_serializer<S>(
    serializer: S,
) -> Result<TestApp<S>, wireframe::WireframeError>
where
    S: TestSerializer + Default,
{
    wireframe::app::WireframeApp::<S, (), Envelope>::with_serializer(serializer)?
        .route(1, Arc::new(|_| Box::pin(async {})))
}

macro_rules! impl_test_serializer_boilerplate {
    ($serializer:ty) => {
        fn serialize<M>(
            &self,
            value: &M,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
        where
            M: wireframe::message::EncodeWith<$serializer>,
        {
            value.encode_with(self)
        }

        fn deserialize<M>(
            &self,
            bytes: &[u8],
        ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
        where
            M: wireframe::message::DecodeWith<$serializer>,
        {
            M::decode_with(self, bytes, &DeserializeContext::empty())
        }
    };
}

#[derive(Default)]
struct CountingSerializer(Arc<AtomicUsize>, Arc<AtomicUsize>);

impl MessageCompatibilitySerializer for CountingSerializer {}

impl Serializer for CountingSerializer {
    impl_test_serializer_boilerplate!(CountingSerializer);

    fn deserialize_with_context<M>(
        &self,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: wireframe::message::DecodeWith<Self>,
    {
        self.1.fetch_add(1, Ordering::Relaxed);
        if context.message_id.is_none() {
            return Err("expected message_id in deserialize context".into());
        }
        M::decode_with(self, bytes, context)
    }
}

impl FrameMetadata for CountingSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        BincodeSerializer.parse(src)
    }
}

#[tokio::test]
async fn metadata_parser_invoked_before_deserialize() -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let deserialize_calls = Arc::new(AtomicUsize::new(0));
    let serializer = CountingSerializer(counter.clone(), deserialize_calls.clone());
    let app = mock_wireframe_app_with_serializer(serializer)?;

    let env = Envelope::new(1, Some(0), vec![42]);

    let out = drive_with_bincode(app, env).await?;
    if out.is_empty() {
        return Err("no frames emitted".into());
    }
    let parse_calls = counter.load(Ordering::Relaxed);
    if parse_calls != 1 {
        return Err(format!("expected 1 parse call, got {parse_calls}").into());
    }
    let actual_deserialize_calls = deserialize_calls.load(Ordering::Relaxed);
    if actual_deserialize_calls != 1 {
        return Err(format!(
            "expected 1 deserialize call with context, got {actual_deserialize_calls}"
        )
        .into());
    }
    Ok(())
}

#[derive(Default)]
struct FallbackSerializer(Arc<AtomicUsize>);

impl MessageCompatibilitySerializer for FallbackSerializer {}

impl Serializer for FallbackSerializer {
    impl_test_serializer_boilerplate!(FallbackSerializer);
}

impl FrameMetadata for FallbackSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, _src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(bincode::error::DecodeError::OtherString("fail".into()))
    }
}

#[tokio::test]
async fn falls_back_to_deserialize_after_parse_error() -> TestResult<()> {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let serializer = FallbackSerializer(parse_calls.clone());
    let app = mock_wireframe_app_with_serializer(serializer)?;

    let env = Envelope::new(1, Some(0), vec![7]);

    let out = drive_with_bincode(app, env).await?;
    if out.is_empty() {
        return Err("no frames emitted".into());
    }
    let actual_parse_calls = parse_calls.load(Ordering::Relaxed);
    if actual_parse_calls != 1 {
        return Err(format!("expected 1 parse call, got {actual_parse_calls}").into());
    }
    Ok(())
}

#[tokio::test]
async fn metadata_is_forwarded_to_deserialize_context() -> TestResult<()> {
    let context_state = Arc::new(Mutex::new(None::<CapturedDeserializeContext>));
    let serializer = ContextCapturingSerializer::new(context_state.clone());
    let app = mock_wireframe_app_with_serializer(serializer)?;

    let envelope = Envelope::new(1, Some(77), vec![1, 2, 3, 4]);
    let expected_parts = envelope.clone().into_parts();
    let output = drive_with_bincode(app, envelope.clone()).await?;
    if output.is_empty() {
        return Err("no frames emitted".into());
    }

    let captured = (*context_state
        .lock()
        .map_err(|_| "mutex poisoned while locking context_state")?)
    .ok_or("expected captured deserialize context")?;

    let expected_message_id = Some(expected_parts.id());
    if captured.message_id != expected_message_id {
        return Err(format!(
            "message id mismatch: expected {expected_message_id:?}, got {:?}",
            captured.message_id
        )
        .into());
    }
    let expected_correlation_id = expected_parts.correlation_id();
    if captured.correlation_id != expected_correlation_id {
        return Err(format!(
            "correlation id mismatch: expected {expected_correlation_id:?}, got {:?}",
            captured.correlation_id
        )
        .into());
    }
    if captured.frame_metadata_len != captured.metadata_bytes_consumed {
        return Err(format!(
            "metadata bytes consumed should match captured frame metadata slice length: metadata \
             length {:?}, bytes consumed {:?}",
            captured.frame_metadata_len, captured.metadata_bytes_consumed
        )
        .into());
    }
    Ok(())
}
