//! Delivery of complete and incremental bodies to a streaming handler.

use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use wireframe::test_helpers;
use wireframe_testing::TestResult;

use super::support::{
    NETWORK_WAIT,
    STREAM_ROUTE,
    assert_body,
    assert_chunks,
    assert_no_reply,
    assert_payload,
    assert_truncated,
    bounded,
    close_client,
    connect,
    finish_server,
    observing_handler,
    progressing_handler,
    read_reply,
    reply_within,
    send_envelope,
    serve_routes,
};

/// A body delivered across two frames reaches the handler over TCP.
#[tokio::test]
async fn streaming_body_reaches_the_handler_over_tcp() -> TestResult {
    let received = Arc::new(Mutex::new(Vec::new()));
    let first_chunk_seen = Arc::new(Notify::new());
    let server = serve_routes(vec![(
        STREAM_ROUTE,
        progressing_handler(Arc::clone(&received), Arc::clone(&first_chunk_seen)),
    )])
    .await?;
    let mut client = connect(server.address).await?;

    let first = test_helpers::first_frame_payload(1.into(), b"alpha", false, None)?;
    let last = test_helpers::continuation_frame_payload(1.into(), 1.into(), b"beta", true)?;
    send_envelope(&mut client, STREAM_ROUTE, &first).await?;
    // The final frame is only sent once the handler has the first chunk, so a
    // passing test proves the body streamed rather than arriving at the end.
    bounded(
        "the handler receiving the first chunk",
        first_chunk_seen.notified(),
    )
    .await?;
    send_envelope(&mut client, STREAM_ROUTE, &last).await?;

    let reply = read_reply(&mut client).await?;
    close_client(&mut client).await?;
    finish_server(server).await?;
    let chunks = received
        .lock()
        .map_err(|_| "handler buffer poisoned")?
        .clone();

    assert_payload(
        &reply,
        b"done",
        "the streaming reply should reach the client",
    );
    assert_body(
        &chunks,
        b"alphabeta",
        "the handler should receive the body in frame order",
    );
    Ok(())
}

/// A single-frame message is delivered as one complete chunk and answered.
///
/// The one-chunk assertion is the point: a message that fits in its first
/// frame must reach the handler immediately, not after a later frame or at end
/// of stream.
#[tokio::test]
async fn a_single_frame_body_arrives_as_one_chunk() -> TestResult {
    let received = Arc::new(Mutex::new(Vec::new()));
    let first_chunk_seen = Arc::new(Notify::new());
    let server = serve_routes(vec![(
        STREAM_ROUTE,
        progressing_handler(Arc::clone(&received), Arc::clone(&first_chunk_seen)),
    )])
    .await?;
    let mut client = connect(server.address).await?;

    let whole = test_helpers::first_frame_payload(2.into(), b"whole", true, None)?;
    send_envelope(&mut client, STREAM_ROUTE, &whole).await?;

    let reply = read_reply(&mut client).await?;
    close_client(&mut client).await?;
    finish_server(server).await?;
    let chunks = received
        .lock()
        .map_err(|_| "handler buffer poisoned")?
        .clone();

    assert_payload(&reply, b"done", "the single-frame body should be answered");
    assert_chunks(
        &chunks,
        &[b"whole".to_vec()],
        "a single-frame body must reach the handler as one chunk",
    );
    Ok(())
}

/// A connection closed mid-body reports `UnexpectedEof` and closes cleanly.
#[tokio::test]
async fn closing_the_connection_mid_body_reports_unexpected_eof() -> TestResult {
    let observed = Arc::new(Mutex::new(None));
    let server = serve_routes(vec![(
        STREAM_ROUTE,
        observing_handler(Arc::clone(&observed)),
    )])
    .await?;
    let mut client = connect(server.address).await?;

    let first = test_helpers::first_frame_payload(3.into(), b"partial", false, None)?;
    send_envelope(&mut client, STREAM_ROUTE, &first).await?;
    close_client(&mut client).await?;
    let has_reply = reply_within(&mut client, NETWORK_WAIT).await?;
    finish_server(server).await?;
    let kind = *observed.lock().map_err(|_| "observation slot poisoned")?;

    assert_truncated(kind);
    assert_no_reply(has_reply);
    Ok(())
}
