//! Tests for correlation id propagation.

use bytes::BytesMut;
use rstest::rstest;
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    frame::{FrameProcessor, LengthPrefixedProcessor},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::drive_with_bincode;

mod correlation_world;
use correlation_world::CorrelationWorld;

#[rstest]
#[tokio::test]
async fn response_echoes_correlation_id() {
    let app = WireframeApp::new()
        .expect("failed to create app")
        .frame_processor(LengthPrefixedProcessor::default())
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))
        .expect("route registration failed");

    let env = Envelope::new(1, 7, vec![1]);
    let out = drive_with_bincode(app, env)
        .await
        .expect("drive_with_bincode failed");

    let mut buf = BytesMut::from(&out[..]);
    let frame = LengthPrefixedProcessor::default()
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (resp, _) = BincodeSerializer
        .deserialize::<Envelope>(&frame)
        .expect("deserialize failed");
    assert_eq!(resp.correlation_id(), 7);
}

#[tokio::test]
#[should_panic(expected = "assertion failed")]
async fn mismatched_correlation_id_panics() {
    let mut world = CorrelationWorld::default();
    world.set_id(7);
    world
        .run_actor_with_frames(
            vec![Envelope::new(1, 7, vec![1]), Envelope::new(1, 9, vec![2])],
            1,
        )
        .await;
    world.assert_ids();
}
