use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::BytesMut;
use wireframe::{
    app::{Envelope, WireframeApp},
    frame::{FrameMetadata, FrameProcessor, LengthPrefixedProcessor},
    serializer::{BincodeSerializer, Serializer},
};

mod util;
use util::run_app_with_frame;

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
    let app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor::default())
        .serializer(serializer)
        .route(1, Arc::new(|_| Box::pin(async {})))
        .unwrap();

    let env = Envelope::new(1, vec![42]);
    let bytes = BincodeSerializer.serialize(&env).unwrap();
    let mut framed = BytesMut::new();
    LengthPrefixedProcessor::default()
        .encode(&bytes, &mut framed)
        .unwrap();

    let out = run_app_with_frame(app, framed.to_vec()).await.unwrap();
    assert!(!out.is_empty());
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}
