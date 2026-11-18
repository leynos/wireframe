//! Peer observation while a live handler forwards its own reply.
//!
//! The reply a streaming handler produces is written after the handler
//! resolves, and a peer that stops reading blocks that write indefinitely. A
//! half-closed peer is only observable by polling the read half during the
//! write, so these tests prove the write is bounded by the peer-close grace.
//!
//! One test covers the reply a handler forwards when it resolves on its own.
//! The other covers the reply a truncated body flushes out: that reply is
//! cancelled with its frame half-written, so the message parked behind it must
//! never be served — its frame would be read as part of the broken stream.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::{io::AsyncReadExt, sync::Notify, time::Instant};
use wireframe_testing::{TestError, TestResult};

use super::watch_support::{
    App,
    Client,
    STEP_WAIT,
    Scenario,
    bounded_for,
    ensure,
    open_body,
    send_complete,
    spawn_connection_buffering,
};
use crate::{
    app::{Envelope, ResponseHandler, StreamingHandler, WireframeApp},
    connection::PEER_CLOSED_RESPONSE_GRACE,
    fragment::FragmentationConfig,
    request::{RequestBodyStream, RequestParts},
    response::Response,
    test_helpers::TestAssembler,
};

/// Route whose streaming handler answers with a payload the peer never reads.
const LIVE_ROUTE: u32 = 72;
/// Buffered route parked behind the live body.
const PARKED_ROUTE: u32 = 73;
/// Message key of the live streaming body.
const LIVE_KEY: u64 = 51;
/// Message key of the buffered message parked behind it.
const PARKED_KEY: u64 = 52;
/// Route whose streaming handler answers only a body the runtime failed.
const EXPIRY_ROUTE: u32 = 74;
/// Message key of the body left to expire.
const EXPIRY_KEY: u64 = 61;
/// Transport buffer for these scenarios, in each direction.
///
/// It is smaller than a reply, so the reply cannot be written in one go and the
/// connection depends on observing the peer to end.
const TRANSPORT_CAPACITY: usize = 64;
/// Reply size, chosen to exceed [`TRANSPORT_CAPACITY`] and still fit one frame.
const BULK_REPLY_BYTES: usize = 512;
/// Assembly timeout for the expiry scenario.
///
/// An assembly ages on the wall clock, which the paused runtime clock cannot
/// advance, so this scenario waits the timeout out for real and keeps it short.
const EXPIRY_TIMEOUT: Duration = Duration::from_millis(20);
/// Real time allowed for the assembly to expire and be reported.
const EXPIRY_SETTLE: Duration = Duration::from_millis(100);

/// Build the reply that outgrows the transport buffer but still fits one frame.
fn bulk_answer() -> Bytes { Bytes::from(vec![7_u8; BULK_REPLY_BYTES]) }

/// Drain a body to its end, reporting whether the peer truncated it.
async fn drain_to_end(mut body: RequestBodyStream) -> bool {
    let mut truncated = false;
    while let Some(item) = body.next().await {
        truncated = truncated || item.is_err();
    }
    truncated
}

/// Handler that answers with a reply too large to buffer.
///
/// `waits_for_truncation` selects the shape each scenario needs. The live-reply
/// scenario must answer before the peer leaves, so it cannot read its body
/// first; the truncated-body scenario answers only a body the peer cut short,
/// so a clean end produces no reply at all.
fn bulk_reply_handler(started: Arc<Notify>, waits_for_truncation: bool) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        let start_signal = Arc::clone(&started);
        Box::pin(async move {
            start_signal.notify_one();
            if waits_for_truncation && !drain_to_end(body).await {
                return None;
            }
            Some(bulk_answer())
        })
    })
}

/// Handler that resolves immediately with a reply too large to buffer.
fn bulk_reply(started: Arc<Notify>) -> StreamingHandler { bulk_reply_handler(started, false) }

/// Handler that answers a truncated body with a reply too large to buffer.
fn truncated_body_reply(started: Arc<Notify>) -> StreamingHandler {
    bulk_reply_handler(started, true)
}

/// Buffered route whose handler records whether it was served at all.
fn recording_reply(served: Arc<AtomicBool>) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        served.store(true, Ordering::SeqCst);
        Box::pin(async { Response::Single(Envelope::new(PARKED_ROUTE, None, vec![1])) })
    })
}

/// Build an application with one bulk-reply streaming route.
fn streaming_app(route: u32, handler: StreamingHandler) -> TestResult<App> {
    WireframeApp::new()
        .map(|app| app.with_message_assembler(TestAssembler))
        .and_then(|app| app.streaming_route(route, handler))
        .map_err(|error| TestError::Msg(format!("build the application: {error}")))
}

/// Build an application whose live handler answers with a bulk reply.
fn bulk_app(started: Arc<Notify>) -> TestResult<App> {
    streaming_app(LIVE_ROUTE, bulk_reply(started))
}

/// Build the two-route application whose truncated body answers anyway.
fn truncated_app(started: Arc<Notify>, served: Arc<AtomicBool>) -> TestResult<App> {
    streaming_app(LIVE_ROUTE, truncated_body_reply(started))?
        .route_response(PARKED_ROUTE, recording_reply(served))
        .map_err(|error| TestError::Msg(format!("build the application: {error}")))
}

/// Build an application whose streaming route answers only a failed body.
fn expiry_app(started: Arc<Notify>) -> TestResult<App> {
    let fragmentation = FragmentationConfig::for_frame_budget(
        1024,
        NonZeroUsize::new(BULK_REPLY_BYTES * 8).unwrap_or(NonZeroUsize::MIN),
        EXPIRY_TIMEOUT,
    );
    streaming_app(EXPIRY_ROUTE, truncated_body_reply(started))
        .map(|app| app.fragmentation(fragmentation))
}

/// Await `connection`, requiring it to end at the peer-close grace.
///
/// The window opens when the peer half-closes, but the grace is armed when the
/// loop observes it, so the upper bound allows a whole step for that to happen.
/// A connection that waits for the peer to finish reading instead never ends.
async fn ends_at_the_grace(
    connection: tokio::task::JoinHandle<std::io::Result<()>>,
    window: Instant,
) -> TestResult {
    let joined = bounded_for(
        PEER_CLOSED_RESPONSE_GRACE + STEP_WAIT,
        "the connection ending after the peer half-closed",
        connection,
    )
    .await?;
    joined??;
    let elapsed = window.elapsed();
    ensure(
        elapsed >= PEER_CLOSED_RESPONSE_GRACE,
        format!("the reply must be given the idle grace: ended after {elapsed:?}"),
    )?;
    ensure(
        elapsed <= PEER_CLOSED_RESPONSE_GRACE + STEP_WAIT,
        format!("the reply must be cancelled at the grace, not later: ended after {elapsed:?}"),
    )
}

/// Read whatever the transport has buffered.
///
/// The reply cannot be written in one go, so bytes arriving here prove the
/// connection is mid-reply. The client reads nothing else, which leaves the
/// write blocked for the half-close that follows.
async fn read_buffered(client: &mut Client) -> TestResult<usize> {
    let mut buffer = [0_u8; TRANSPORT_CAPACITY];
    let read = bounded_for(
        STEP_WAIT,
        "the reply write reaching the transport",
        client.get_mut().read(&mut buffer),
    )
    .await??;
    Ok(read)
}

/// A live handler's own reply is bounded when the peer stops reading.
///
/// The handler resolves before the peer leaves, so the reply is already being
/// written when the half-close arrives. Nothing but observing the read half
/// during that write can end the connection.
#[tokio::test(start_paused = true)]
async fn a_live_reply_is_bounded_when_the_peer_stops_reading() -> TestResult {
    let started = Arc::new(Notify::new());
    let Scenario {
        mut client,
        connection,
    } = spawn_connection_buffering(bulk_app(Arc::clone(&started))?, TRANSPORT_CAPACITY).await?;

    open_body(&mut client, LIVE_ROUTE, LIVE_KEY, &started).await?;
    ensure(
        read_buffered(&mut client).await? > 0,
        "the handler's reply must reach the transport before the peer leaves",
    )?;
    SinkExt::<Bytes>::close(&mut client).await?;
    let window = Instant::now();
    ends_at_the_grace(connection, window).await
}

/// A truncated body's reply stops the parked drain.
///
/// The reply is cancelled while its frame is half-written. Serving the parked
/// message after that would make its frame read as part of the broken stream,
/// so the drain must not start at all.
#[tokio::test(start_paused = true)]
async fn a_cancelled_live_reply_does_not_drain_parked_messages() -> TestResult {
    let served = Arc::new(AtomicBool::new(false));
    let started = Arc::new(Notify::new());
    let Scenario {
        mut client,
        connection,
    } = spawn_connection_buffering(
        truncated_app(Arc::clone(&started), Arc::clone(&served))?,
        TRANSPORT_CAPACITY,
    )
    .await?;

    open_body(&mut client, LIVE_ROUTE, LIVE_KEY, &started).await?;
    send_complete(&mut client, PARKED_ROUTE, PARKED_KEY, b"parked").await?;
    // The half-close truncates the live body, whose handler answers with a
    // reply the peer never reads.
    SinkExt::<Bytes>::close(&mut client).await?;
    let window = Instant::now();
    ends_at_the_grace(connection, window).await?;

    ensure(
        !served.load(Ordering::SeqCst),
        "a cancelled reply must stop the parked drain",
    )
}

/// An expired body's reply is bounded when the peer stops reading.
///
/// No further frames arrive, so the assembly expires and the runtime fails the
/// body. The handler answers that failure with a reply the peer never reads,
/// and nothing but observing the read half during that write can end the
/// connection.
#[tokio::test(start_paused = true)]
async fn an_expired_body_reply_is_bounded_when_the_peer_stops_reading() -> TestResult {
    let started = Arc::new(Notify::new());
    let Scenario {
        mut client,
        connection,
    } = spawn_connection_buffering(expiry_app(Arc::clone(&started))?, TRANSPORT_CAPACITY).await?;

    open_body(&mut client, EXPIRY_ROUTE, EXPIRY_KEY, &started).await?;
    // The assembly ages on the wall clock, not the paused runtime clock, so the
    // timeout has to pass for real. The loop's read wake-up then reports it.
    std::thread::sleep(EXPIRY_SETTLE);
    ensure(
        read_buffered(&mut client).await? > 0,
        "the expired body's reply must reach the transport before the peer leaves",
    )?;
    SinkExt::<Bytes>::close(&mut client).await?;
    let window = Instant::now();
    ends_at_the_grace(connection, window).await
}
