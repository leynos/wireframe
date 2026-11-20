//! Peer observation while a body chunk waits for admission.
//!
//! A chunk that cannot be admitted keeps the connection loop off the reader:
//! the byte budget stays saturated for as long as the handler declines to take
//! its chunk, so nothing else in the loop would ever notice the peer leaving.
//! Without observing the read half during the admission, such a connection
//! outlives the peer that abandoned it and holds its handler, its queued bytes
//! and its server drain with it.
//!
//! The clock is paused, so the bounded waits under test cost no real time and
//! the assertion is about the path taken rather than about how long it took.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::SinkExt;
use tokio::sync::Notify;
use wireframe_testing::{TestError, TestResult};

#[cfg(feature = "metrics")]
use super::super::stream_outcome_support::{paused_block_on, recorded};
use super::watch_support::{
    App,
    STEP_WAIT,
    Scenario,
    bounded,
    bounded_for,
    ensure,
    read_reply,
    send_complete,
    send_request,
    spawn_connection,
};
#[cfg(feature = "metrics")]
use crate::metrics::StreamingBodyOutcome;
use crate::{
    app::{BudgetBytes, Envelope, MemoryBudgets, ResponseHandler, StreamingHandler, WireframeApp},
    request::{RequestBodyStream, RequestParts},
    response::Response,
    test_helpers::{TestAssembler, continuation_frame_payload, first_frame_payload},
};

/// Route whose streaming handler never reads its body.
const STALLED_ROUTE: u32 = 75;
/// Message key of the stalled streaming body.
const STALLED_KEY: u64 = 62;
/// Buffered route that answers at once, proving the loop still serves.
const PROBE_ROUTE: u32 = 76;
/// Message key of the probe request.
const PROBE_KEY: u64 = 63;
/// Reply the probe route returns.
const PROBE_REPLY: &[u8] = b"served";
/// Size of each body chunk, small enough to fit the default frame budget.
const CHUNK_BYTES: usize = 256;
/// Per-message cap, generous enough that the message never trips it.
const MESSAGE_BUDGET: usize = 64 * 1024;
/// Per-connection cap that binds.
///
/// Two chunks exceed it, so the second one cannot be admitted while the first
/// is still charged; a single chunk fits comfortably inside it, so the body is
/// able to start.
const CONNECTION_BUDGET: usize = 500;
/// Bound for the connection ending once the peer has gone.
///
/// The teardown bound is a whole read timeout, so a little slack is allowed on
/// top for scheduling at one virtual instant.
const END_WAIT: Duration = Duration::from_millis(200);

/// Handler that announces itself and then never touches its body.
///
/// Holding the stream without reading it is what keeps the first chunk charged
/// to the connection budget, so every later chunk is refused admission and the
/// connection always has one waiting when the peer leaves.
fn stalled_handler(
    started: Arc<Notify>,
    release: Arc<Notify>,
    read: Arc<AtomicUsize>,
) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        let start_signal = Arc::clone(&started);
        let release_signal = Arc::clone(&release);
        let counted = Arc::clone(&read);
        Box::pin(async move {
            start_signal.notify_one();
            // Never notified: the handler stays parked for the whole scenario.
            release_signal.notified().await;
            counted.fetch_add(1, Ordering::SeqCst);
            drop(body);
            None
        })
    })
}

/// Buffered route whose handler answers at once.
///
/// The reply is the probe: it can only be written once the stalled body has
/// been failed and the frame buffered behind it dispatched.
fn probe_reply() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async { Response::Single(Envelope::new(PROBE_ROUTE, None, PROBE_REPLY.to_vec())) })
    })
}

/// Convert a byte count into a budget.
fn budget(bytes: usize) -> TestResult<BudgetBytes> {
    NonZeroUsize::new(bytes)
        .map(BudgetBytes::new)
        .ok_or_else(|| format!("a budget of {bytes} bytes must be non-zero").into())
}

/// Build the application every scenario drives.
fn stalled_app(handler: StreamingHandler) -> TestResult<App> {
    let budgets = MemoryBudgets::new(
        budget(MESSAGE_BUDGET)?,
        budget(CONNECTION_BUDGET)?,
        budget(CONNECTION_BUDGET)?,
    );
    WireframeApp::new()
        .map(|app| app.with_message_assembler(TestAssembler))
        .and_then(|app| app.streaming_route(STALLED_ROUTE, handler))
        .and_then(|app| app.route_response(PROBE_ROUTE, probe_reply()))
        .map(|app| app.memory_budgets(budgets))
        .map_err(|error| TestError::Msg(format!("build the application: {error}")))
}

/// A peer that leaves while a chunk waits for admission ends the connection.
///
/// The handler never reads, so the first chunk stays charged and the second is
/// refused admission for as long as the connection lasts. Nothing in the loop
/// makes progress except the read half, and only observing it can end the
/// connection.
#[tokio::test(start_paused = true)]
async fn a_peer_leaving_while_a_chunk_waits_is_observed() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let read = Arc::new(AtomicUsize::new(0));
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(stalled_app(stalled_handler(
        Arc::clone(&started),
        Arc::clone(&release),
        Arc::clone(&read),
    ))?)
    .await?;

    let chunk = vec![7_u8; CHUNK_BYTES];
    send_request(
        &mut client,
        STALLED_ROUTE,
        &first_frame_payload(STALLED_KEY.into(), &chunk, false, None)?,
    )
    .await?;
    bounded("the live handler starting", started.notified()).await?;
    // The second chunk cannot be admitted while the first is charged, so the
    // connection is left holding one pending admission.
    send_request(
        &mut client,
        STALLED_ROUTE,
        &continuation_frame_payload(STALLED_KEY.into(), 1_u32.into(), &chunk, false)?,
    )
    .await?;
    SinkExt::<Bytes>::close(&mut client).await?;

    let joined = bounded_for(
        STEP_WAIT + END_WAIT,
        "the connection ending after the peer left",
        connection,
    )
    .await?;
    joined??;
    ensure(
        read.load(Ordering::SeqCst) == 0,
        "the stalled handler must not be released",
    )
}

/// A stalled final chunk must not carry the connection past its budget.
///
/// A pending chunk that ends the body is offered to the handler when the
/// exchange closes, and that delivery is metered like any other. The first
/// chunk is still charged and the final chunk would not fit, so the delivery
/// waits; admitting it anyway would put the connection over its aggregate
/// limit and report a body as delivered that the bounded wait then drops.
///
/// The connection keeps running after that close, because a further
/// continuation is buffered behind the stalled chunk and the probe behind
/// that. The loop must reach both frames and end cleanly rather than abort
/// through a budget breach the delivery caused.
///
/// The outcome is read from a private recorder, because the abandoned chunk is
/// dropped with the handler and leaves nothing on the wire to inspect.
#[cfg(feature = "metrics")]
#[test]
fn a_stalled_final_chunk_ends_its_body_within_budget() -> TestResult {
    let (scenario, counts) = recorded(|| paused_block_on(stalled_final_chunk_scenario()));
    scenario??;
    ensure(
        counts.count(StreamingBodyOutcome::Delivered) == 0,
        "a final chunk the budget refuses must not be counted as delivered",
    )?;
    ensure(
        counts.count(StreamingBodyOutcome::Abandoned) > 0,
        "the body the refused final chunk ended must be counted as abandoned",
    )
}

/// Drive the stalled-final-chunk scenario on its recorded runtime.
///
/// The handler is never released, so the connection ends with the body it was
/// still waiting on; the peer half-closes before the reply is due, which is
/// what closes the exchange while the final chunk waits for admission.
#[cfg(feature = "metrics")]
async fn stalled_final_chunk_scenario() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let read = Arc::new(AtomicUsize::new(0));
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(stalled_app(stalled_handler(
        Arc::clone(&started),
        Arc::clone(&release),
        Arc::clone(&read),
    ))?)
    .await?;

    let chunk = vec![7_u8; CHUNK_BYTES];
    send_request(
        &mut client,
        STALLED_ROUTE,
        &first_frame_payload(STALLED_KEY.into(), &chunk, false, None)?,
    )
    .await?;
    bounded("the live handler starting", started.notified()).await?;
    // This chunk ends the body, and cannot be admitted while the first is
    // charged, so closing the exchange has to decide its fate.
    send_request(
        &mut client,
        STALLED_ROUTE,
        &continuation_frame_payload(STALLED_KEY.into(), 1_u32.into(), &chunk, true)?,
    )
    .await?;
    // Buffered while the final chunk waits, so a step still runs after the
    // stalled body closes and the memory budget is checked again.
    send_request(
        &mut client,
        STALLED_ROUTE,
        &continuation_frame_payload(STALLED_KEY.into(), 2_u32.into(), &chunk, false)?,
    )
    .await?;
    send_complete(&mut client, PROBE_ROUTE, PROBE_KEY, b"probe").await?;
    SinkExt::<Bytes>::close(&mut client).await?;

    let joined = bounded_for(
        STEP_WAIT + END_WAIT,
        "the connection ending after the stalled final chunk",
        connection,
    )
    .await?;
    joined??;
    ensure(
        read.load(Ordering::SeqCst) == 0,
        "the stalled handler must not be released",
    )
}

/// A frame buffered behind a stalled chunk cannot leave the connection blind.
///
/// The first chunk stays charged, so the second is refused admission and stays
/// queued. A third continuation is then read while that admission waits, and a
/// buffered frame keeps the loop off the reader: the third frame would never be
/// dispatched, and the peer's later close would never be observed, while the
/// chunk cannot settle as long as the handler declines to read.
///
/// The admission wait is the only exit left. Expiring it fails the stalled
/// body, which abandons the queued chunk and leaves the buffered continuation
/// belonging to no body. The step then falls through to dispatch that frame and
/// serve the probe behind it, proving the loop recovered rather than spinning.
#[tokio::test(start_paused = true)]
async fn a_chunk_stalled_behind_a_buffered_frame_ends_its_body() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let read = Arc::new(AtomicUsize::new(0));
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(stalled_app(stalled_handler(
        Arc::clone(&started),
        Arc::clone(&release),
        Arc::clone(&read),
    ))?)
    .await?;

    let chunk = vec![7_u8; CHUNK_BYTES];
    send_request(
        &mut client,
        STALLED_ROUTE,
        &first_frame_payload(STALLED_KEY.into(), &chunk, false, None)?,
    )
    .await?;
    bounded("the live handler starting", started.notified()).await?;
    send_request(
        &mut client,
        STALLED_ROUTE,
        &continuation_frame_payload(STALLED_KEY.into(), 1_u32.into(), &chunk, false)?,
    )
    .await?;
    // Read while the second chunk waits for admission, so it is buffered
    // rather than dispatched. The probe behind it stays in the socket until
    // the loop reaches the reader again.
    send_request(
        &mut client,
        STALLED_ROUTE,
        &continuation_frame_payload(STALLED_KEY.into(), 2_u32.into(), &chunk, false)?,
    )
    .await?;
    send_complete(&mut client, PROBE_ROUTE, PROBE_KEY, b"probe").await?;

    let reply = read_reply(&mut client, "the probe reply").await?;
    ensure(
        reply == PROBE_REPLY,
        format!("the probe must be served once the stalled body fails, got {reply:?}"),
    )?;
    ensure(
        read.load(Ordering::SeqCst) == 0,
        "the stalled handler must not be released",
    )?;

    SinkExt::<Bytes>::close(&mut client).await?;
    let joined = bounded_for(
        STEP_WAIT + END_WAIT,
        "the connection ending after the peer left",
        connection,
    )
    .await?;
    joined??;
    Ok(())
}
