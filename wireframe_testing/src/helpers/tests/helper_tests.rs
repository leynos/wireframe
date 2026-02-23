use std::{io, sync::Arc};

use futures::future::BoxFuture;
use wireframe::{
    Serializer,
    app::{Envelope, WireframeApp},
    serializer::BincodeSerializer,
};

use crate::helpers::{MAX_CAPACITY, decode_frames, drive_with_payloads, run_app};

#[tokio::test]
async fn run_app_rejects_zero_capacity() {
    let app: WireframeApp<BincodeSerializer, (), Envelope> =
        WireframeApp::new().expect("failed to create app");
    let err = run_app(app, vec![], Some(0))
        .await
        .expect_err("capacity of zero should error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test]
async fn run_app_rejects_excess_capacity() {
    let app: WireframeApp<BincodeSerializer, (), Envelope> =
        WireframeApp::new().expect("failed to create app");
    let err = run_app(app, vec![], Some(MAX_CAPACITY + 1))
        .await
        .expect_err("capacity beyond max should error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test]
async fn drive_with_payloads_wraps_frames() -> io::Result<()> {
    let app: WireframeApp<BincodeSerializer, (), Envelope> =
        WireframeApp::new().expect("failed to create app");
    let app = app
        .route(
            1,
            Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) }),
        )
        .expect("route registration should succeed");
    let serializer = BincodeSerializer;
    let payload = vec![1_u8, 2, 3];
    let env = Envelope::new(1, Some(7), payload.clone());
    let encoded = serializer
        .serialize(&env)
        .expect("failed to serialize envelope");

    let out = drive_with_payloads(app, vec![encoded]).await?;
    let frames = decode_frames(out);
    let [first] = frames.as_slice() else {
        panic!("expected a single response frame");
    };
    let (decoded, _) = serializer
        .deserialize::<Envelope>(first)
        .expect("failed to deserialise envelope");
    assert_eq!(
        decoded.payload_bytes(),
        payload.as_slice(),
        "payload mismatch"
    );
    Ok(())
}
