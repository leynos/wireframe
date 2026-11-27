//! Parking of a complete message behind the live streaming body.
//!
//! A connection delivers one request body at a time. A message that completes
//! while another body owns the dispatch slot is parked and delivered once the
//! slot frees, and these tests observe that through the socket: the peer is not
//! answered early, and the parked reply arrives after the live one.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::Notify;
use wireframe::{
    app::StreamingHandler,
    request::{RequestBodyStream, RequestParts},
    test_helpers,
};
use wireframe_testing::TestResult;

use super::support::{
    LIVE_KEY,
    STREAM_ROUTE,
    assert_chunks,
    assert_not_answered_early,
    assert_payload,
    bounded,
    close_client,
    connect,
    finish_server,
    read_reply,
    recording_handler,
    reply_within,
    send_envelope,
    serve_routes,
};

/// Route identifier for the message parked behind a live body.
const PARKED_ROUTE: u32 = 91;
/// Message key of the message parked behind the live body.
const PARKED_KEY: u64 = 22;
/// How long a parked message is given to answer before it should.
const SETTLE_WAIT: std::time::Duration = std::time::Duration::from_millis(300);

/// Handler that holds its body open until released, then replies.
fn holding_handler(started: Arc<Notify>, release: Arc<Notify>) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, mut body: RequestBodyStream| {
        let start_signal = Arc::clone(&started);
        let release_signal = Arc::clone(&release);
        Box::pin(async move {
            start_signal.notify_one();
            release_signal.notified().await;
            // The live handler only has to keep its body open; a read error is
            // the end of the exchange either way.
            while body.next().await.is_some() {}
            Some(Bytes::from_static(b"live"))
        })
    })
}

/// A message parked behind a live body is answered once the slot frees.
///
/// The parked message is a *complete* message for a streaming route, so it
/// exercises the pre-filled delivery path on a real connection: the handler is
/// started with a body channel that shares the connection's byte account, and
/// the whole body is queued as its only chunk.
#[tokio::test]
async fn a_message_parked_behind_a_live_body_is_answered_after_it_finishes() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let parked_chunks = Arc::new(Mutex::new(Vec::new()));
    let server = serve_routes(vec![
        (
            STREAM_ROUTE,
            holding_handler(Arc::clone(&started), Arc::clone(&release)),
        ),
        (PARKED_ROUTE, recording_handler(Arc::clone(&parked_chunks))),
    ])
    .await?;
    let mut client = connect(server.address).await?;

    let first = test_helpers::first_frame_payload(LIVE_KEY.into(), b"start", false, None)?;
    send_envelope(&mut client, STREAM_ROUTE, &first).await?;
    bounded("the live handler starting", started.notified()).await?;

    let whole = test_helpers::first_frame_payload(PARKED_KEY.into(), b"whole", true, None)?;
    send_envelope(&mut client, PARKED_ROUTE, &whole).await?;
    assert_not_answered_early(reply_within(&mut client, SETTLE_WAIT).await?);

    // Release the live handler before completing its body: it must be draining
    // when the final frame arrives, exactly as a real upload would be.
    release.notify_one();
    let last = test_helpers::continuation_frame_payload(LIVE_KEY.into(), 1.into(), b"end", true)?;
    send_envelope(&mut client, STREAM_ROUTE, &last).await?;
    let live_reply = read_reply(&mut client).await?;
    let parked_reply = read_reply(&mut client).await?;
    close_client(&mut client).await?;
    finish_server(server).await?;
    let chunks = parked_chunks
        .lock()
        .map_err(|_| "handler buffer poisoned")?
        .clone();

    assert_payload(
        &live_reply,
        b"live",
        "the live body should be answered first",
    );
    assert_payload(
        &parked_reply,
        b"done",
        "the parked message should be answered once the slot frees",
    );
    assert_chunks(
        &chunks,
        &[b"whole".to_vec()],
        "a pre-filled body must reach the parked handler as one chunk",
    );
    Ok(())
}
