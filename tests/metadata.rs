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
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn metadata_parser_invoked_before_deserialize() -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let deserialize_calls = Arc::new(AtomicUsize::new(0));
    let serializer = CountingSerializer(counter.clone(), deserialize_calls.clone());
    let app = mock_wireframe_app_with_serializer(serializer)?;

    let env = Envelope::new(1, Some(0), vec![42]);

    let out = drive_with_bincode(app, env).await?;
    assert!(!out.is_empty(), "no frames emitted");
    assert_eq!(counter.load(Ordering::Relaxed), 1, "expected 1 parse call");
    assert_eq!(
        deserialize_calls.load(Ordering::Relaxed),
        1,
        "expected 1 deserialize call with context"
    );
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
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn falls_back_to_deserialize_after_parse_error() -> TestResult<()> {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let serializer = FallbackSerializer(parse_calls.clone());
    let app = mock_wireframe_app_with_serializer(serializer)?;

    let env = Envelope::new(1, Some(0), vec![7]);

    let out = drive_with_bincode(app, env).await?;
    assert!(!out.is_empty(), "no frames emitted");
    assert_eq!(
        parse_calls.load(Ordering::Relaxed),
        1,
        "expected 1 parse call"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CapturedContext {
    message_id: Option<u32>,
    correlation_id: Option<u64>,
    metadata_bytes_consumed: Option<usize>,
    frame_metadata_len: Option<usize>,
}

#[derive(Default)]
struct ContextCapturingSerializer {
    captured: Arc<Mutex<Option<CapturedContext>>>,
}

impl ContextCapturingSerializer {
    fn capture_handle(&self) -> Arc<Mutex<Option<CapturedContext>>> { self.captured.clone() }
}

impl MessageCompatibilitySerializer for ContextCapturingSerializer {}

impl Serializer for ContextCapturingSerializer {
    impl_test_serializer_boilerplate!(ContextCapturingSerializer);

    fn deserialize_with_context<M>(
        &self,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: wireframe::message::DecodeWith<Self>,
    {
        let captured = CapturedContext {
            message_id: context.message_id,
            correlation_id: context.correlation_id,
            metadata_bytes_consumed: context.metadata_bytes_consumed,
            frame_metadata_len: context.frame_metadata.map(<[u8]>::len),
        };
        let mut state = self.captured.lock().map_err(|_| {
            "ContextCapturingSerializer::deserialize_with_context captured mutex poisoned"
        })?;
        *state = Some(captured);
        M::decode_with(self, bytes, context)
    }
}

impl FrameMetadata for ContextCapturingSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        BincodeSerializer.parse(src)
    }
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn metadata_is_forwarded_to_deserialize_context() -> TestResult<()> {
    let serializer = ContextCapturingSerializer::default();
    let context_state = serializer.capture_handle();
    let app = mock_wireframe_app_with_serializer(serializer)?;

    let envelope = Envelope::new(1, Some(77), vec![1, 2, 3, 4]);
    let expected_parts = envelope.clone().into_parts();
    let output = drive_with_bincode(app, envelope.clone()).await?;
    assert!(!output.is_empty(), "no frames emitted");

    let captured = (*context_state
        .lock()
        .unwrap_or_else(|_| panic!("mutex poisoned while locking context_state")))
    .ok_or("expected captured deserialize context")?;

    assert_eq!(captured.message_id, Some(expected_parts.id()));
    assert_eq!(captured.correlation_id, expected_parts.correlation_id());
    assert_eq!(
        captured.frame_metadata_len, captured.metadata_bytes_consumed,
        "metadata bytes consumed should match captured frame metadata slice length"
    );
    Ok(())
}
