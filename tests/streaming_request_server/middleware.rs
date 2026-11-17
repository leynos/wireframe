//! Proof that streaming routes bypass the middleware chain.
//!
//! Middleware transforms a whole payload, so it cannot run around a handler
//! that owns a body incrementally. A streaming route is therefore served
//! directly, while buffered routes on the same connection still run the chain.
//! The test interleaves both on one connection so the bypass is shown to be a
//! property of the route, not of the application or the connection.

use std::sync::{
    Arc,
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use wireframe::app::{Envelope, Handler, WireframeApp};
use wireframe_testing::TestResult;

use super::{
    support::{
        BUFFER_CAPACITY,
        BUFFER_ROUTE,
        STREAM_ROUTE,
        TestApp,
        assert_chunks,
        assert_count,
        assert_payload,
        close_client,
        connect,
        finish_server,
        read_reply,
        recording_handler,
        send_envelope,
        serve_with,
    },
    tagging::TransformCountingMiddleware,
};

/// The tag the middleware appends to every frame it processes.
const TAG: u8 = b'A';

/// A handler that accepts an envelope without changing it.
fn echo_handler() -> Handler<Envelope> { Arc::new(|_: &Envelope| Box::pin(async {})) }

/// Streaming routes are served directly; buffered routes run the chain.
#[tokio::test]
async fn a_streaming_route_bypasses_the_middleware_chain() -> TestResult {
    let transforms = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(Mutex::new(Vec::new()));
    let server = serve_with({
        let transform_count = Arc::clone(&transforms);
        let recorded = Arc::clone(&received);
        move || {
            // No message assembler here: a streaming route then receives its
            // whole payload as one chunk, which is the simplest body a
            // middleware transform could have touched.
            let app: TestApp = WireframeApp::new()?
                .buffer_capacity(BUFFER_CAPACITY)
                .route(BUFFER_ROUTE, echo_handler())?
                .streaming_route(STREAM_ROUTE, recording_handler(Arc::clone(&recorded)))?
                .wrap(TransformCountingMiddleware {
                    tag: TAG,
                    transforms: Arc::clone(&transform_count),
                })?;
            Ok(app)
        }
    })
    .await?;
    let mut client = connect(server.address).await?;

    send_envelope(&mut client, STREAM_ROUTE, b"Y").await?;
    let stream_reply = read_reply(&mut client).await?;
    send_envelope(&mut client, BUFFER_ROUTE, b"X").await?;
    let buffered_reply = read_reply(&mut client).await?;
    close_client(&mut client).await?;
    finish_server(server).await?;
    let chunks = received
        .lock()
        .map_err(|_| "handler buffer poisoned")?
        .clone();

    assert_chunks(
        &chunks,
        &[b"Y".to_vec()],
        "middleware must not touch the streaming request body",
    );
    assert_payload(
        &stream_reply,
        b"done",
        "the streaming reply must reach the client untagged",
    );
    assert_payload(
        &buffered_reply,
        b"XAA",
        "the buffered control must show both the request and response tag",
    );
    assert_count(
        transforms.load(Ordering::SeqCst),
        1,
        "the middleware chain should have been prepared once for the connection",
    );
    Ok(())
}
