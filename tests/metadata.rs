//! Tests for frame metadata parsing using custom serializers.
//!
//! They ensure parse callbacks run before deserialization and errors fall back correctly.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use wireframe::{
    app::{Envelope, WireframeApp},
    frame::{FrameMetadata, LengthPrefixedProcessor},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestSerializer, drive_with_bincode};

fn mock_wireframe_app_with_serializer<S>(serializer: S) -> WireframeApp<S>
where
    S: TestSerializer,
{
    WireframeApp::new()
        .expect("failed to create app")
        .frame_processor(LengthPrefixedProcessor::default())
        .serializer(serializer)
        .route(1, Arc::new(|_| Box::pin(async {})))
        .expect("route registration failed")
}

struct CountingSerializer(Arc<AtomicUsize>);

impl Serializer for CountingSerializer {
    fn serialize<M: wireframe::message::Message>(
        &self,
        value: &M,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        BincodeSerializer.serialize(value)
    }

    fn deserialize<M: wireframe::message::Message>(
        &self,
        _bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>> {
        panic!("unexpected deserialize call")
    }
}

impl FrameMetadata for CountingSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        BincodeSerializer.parse(src)
    }
}

#[tokio::test]
async fn metadata_parser_invoked_before_deserialize() {
    let counter = Arc::new(AtomicUsize::new(0));
    let serializer = CountingSerializer(counter.clone());
    let app = mock_wireframe_app_with_serializer(serializer);

    let env = Envelope::new(1, 0, vec![42]);

    let out = drive_with_bincode(app, env)
        .await
        .expect("drive_with_bincode failed");
    assert!(!out.is_empty());
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

struct FallbackSerializer(Arc<AtomicUsize>, Arc<AtomicUsize>);

impl Serializer for FallbackSerializer {
    fn serialize<M: wireframe::message::Message>(
        &self,
        value: &M,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        BincodeSerializer.serialize(value)
    }

    fn deserialize<M: wireframe::message::Message>(
        &self,
        bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        BincodeSerializer.deserialize(bytes)
    }
}

impl FrameMetadata for FallbackSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, _src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(bincode::error::DecodeError::OtherString("fail".into()))
    }
}

#[tokio::test]
async fn falls_back_to_deserialize_after_parse_error() {
    let parse_calls = Arc::new(AtomicUsize::new(0));
    let deser_calls = Arc::new(AtomicUsize::new(0));
    let serializer = FallbackSerializer(parse_calls.clone(), deser_calls.clone());
    let app = mock_wireframe_app_with_serializer(serializer);

    let env = Envelope::new(1, 0, vec![7]);

    let out = drive_with_bincode(app, env)
        .await
        .expect("drive_with_bincode failed");
    assert!(!out.is_empty());
    assert_eq!(parse_calls.load(Ordering::SeqCst), 1);
    assert_eq!(deser_calls.load(Ordering::SeqCst), 1);
}
