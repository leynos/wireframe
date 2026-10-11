//! End-to-end TCP coverage for immutable prepared applications.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[path = "common/fallible_assertions/check_equal.rs"]
mod fallible_check_equal;

use fallible_check_equal::check_equal;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use wireframe::{
    app::{Envelope, WireframeApp},
    serializer::{BincodeSerializer, Serializer},
    test_helpers::connection_startup::{TransformCountingMiddleware, noop_handler},
};
use wireframe_testing::{TestResult, decode_frames, encode_frame};

type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Encode an envelope into the default transport frame.
fn build_frame(payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let envelope = Envelope::new(1, Some(7), payload);
    let payload = serializer.serialize(&envelope)?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, payload)?)
}

/// Decode the response envelope and return its payload.
fn response_payload(bytes: &[u8]) -> TestResult<Vec<u8>> {
    let frames = decode_frames(bytes)?;
    let [frame] = frames.as_slice() else {
        return Err("expected one response frame".into());
    };
    let serializer = BincodeSerializer;
    let (response, _) = serializer.deserialize::<Envelope>(frame)?;
    Ok(wireframe::app::Packet::into_parts(response).into_payload())
}

/// Prepared applications serve TCP connections without rebuilding middleware.
#[tokio::test]
async fn prepared_app_serves_tcp_connection_without_retransforming() -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let prepared = TestApp::new()?
        .route(1, noop_handler())?
        .wrap(TransformCountingMiddleware::new(
            b'A',
            Arc::clone(&transforms),
        ))?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    check_equal(
        &transforms.load(Ordering::SeqCst),
        &1,
        "preparation should transform the route once",
    )?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        prepared.handle_connection_result(stream).await
    });

    let mut client = TcpStream::connect(address).await?;
    client.write_all(&build_frame(vec![b'X'])?).await?;
    client.shutdown().await?;
    let mut response = Vec::new();
    client.read_to_end(&mut response).await?;
    server.await??;

    check_equal(
        &response_payload(&response)?,
        b"XAA",
        "the TCP response should carry both middleware tags",
    )?;
    check_equal(
        &transforms.load(Ordering::SeqCst),
        &1,
        "serving the TCP connection should not rebuild the route",
    )?;
    Ok(())
}
