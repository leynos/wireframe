//! Connection-level back-pressure from a slow streaming handler.
//!
//! The dispatch slot admits body chunks only while the bounded body channel
//! and the connection's byte budget allow it. When neither does, the
//! connection stops reading, so a peer that keeps sending eventually blocks in
//! `write` instead of the server buffering without limit. These tests observe
//! that behaviour through the socket, not by inspecting internal counters.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use tokio::{
    sync::{Notify, oneshot},
    time::timeout,
};
use wireframe::{
    app::{BudgetBytes, MemoryBudgets, StreamingHandler},
    request::{RequestBodyStream, RequestParts},
    test_helpers,
};
use wireframe_testing::TestResult;

use super::support::{
    AppLimits,
    Client,
    STREAM_ROUTE,
    assert_blocked,
    assert_chunks,
    assert_count,
    assert_not_answered_early,
    assert_payload,
    bounded,
    bounded_for,
    close_client,
    connect,
    finish_server,
    read_reply,
    recording_handler,
    reply_within,
    send_envelope,
    serve_routes,
    serve_routes_within,
};

/// Route identifier for the message parked behind a live body.
const PARKED_ROUTE: u32 = 91;
/// Message key of the live streaming body.
const LIVE_KEY: u64 = 21;
/// Message key of the message parked behind it.
const PARKED_KEY: u64 = 22;

/// Largest frame the stall scenario's codec accepts.
///
/// It has to carry the scenario's chunks, and the per-message cap derived from
/// it has to admit the whole body: otherwise a size breach, not back-pressure,
/// ends the message.
const PRESSURE_FRAME_BUDGET: usize = 4 * 1024 * 1024;
/// Per-message cap for the stall scenario, well above the whole body.
const PRESSURE_MESSAGE_BUDGET: usize = 64 * 1024 * 1024;
/// Bytes the connection may retain once the handler stops consuming.
///
/// This is the budget that binds: a stalled handler leaves at most these bytes
/// queued, and the connection then stops reading.
const PRESSURE_CONNECTION_BUDGET: usize = 64 * 1024;
/// Size of each bulk body chunk, well inside any supported frame limit.
const BULK_CHUNK: usize = 32 * 1024;
/// Width of the decimal sequence stamp at the start of every chunk.
const STAMP_BYTES: usize = 4;
/// Continuation frames a stalled handler must apply pressure against.
///
/// The body is far larger than the connection budget plus any plausible kernel
/// socket buffering, so a peer that is not blocked finishes writing it well
/// inside the observation window.
const BULK_FRAMES: u32 = 1024;
/// Chunks the handler should eventually see: first frame, bulk, and last.
const TOTAL_CHUNKS: usize = BULK_FRAMES as usize + 2;
/// How long a sender that should be blocked is given to prove otherwise.
///
/// The bound only has to exceed the time an unblocked peer would need to write
/// thirty-two megabytes over loopback, so a machine under load can still reach
/// the assertion; a blocked peer never reaches it at all.
const PRESSURE_WAIT: Duration = Duration::from_millis(500);
/// How long the release-to-reply exchange may take.
///
/// Draining the same thirty-two megabytes through a small queue is slower than
/// one network step, but it is still bounded, so a stuck connection fails the
/// test instead of hanging it.
const DRAIN_WAIT: Duration = Duration::from_secs(10);
/// How long a parked message is given to answer before it should.
const SETTLE_WAIT: Duration = Duration::from_millis(300);

/// Return the stall scenario's connection budgets.
///
/// The per-message cap stays generous so the message can complete, while the
/// connection and in-flight budgets are small enough that a stalled handler
/// leaves the reader with nowhere to put the next chunk.
fn pressure_budgets() -> TestResult<MemoryBudgets> {
    let message = budget_bytes(PRESSURE_MESSAGE_BUDGET)?;
    let connection = budget_bytes(PRESSURE_CONNECTION_BUDGET)?;
    Ok(MemoryBudgets::new(message, connection, connection))
}

/// Convert a byte count into a budget, rejecting a zero count.
fn budget_bytes(bytes: usize) -> TestResult<BudgetBytes> {
    NonZeroUsize::new(bytes)
        .map(BudgetBytes::new)
        .ok_or_else(|| format!("a budget of {bytes} bytes must be non-zero").into())
}

/// Stamp a chunk with the sequence number of the frame carrying it.
///
/// Every chunk has the same length and contents, so only an explicit stamp can
/// show whether the handler received them in the order the peer sent them.
fn stamped(mut chunk: Vec<u8>, sequence: u32) -> Vec<u8> {
    let stamp = format!("{sequence:04}");
    if let Some(field) = chunk.get_mut(..STAMP_BYTES) {
        field.copy_from_slice(stamp.as_bytes());
    }
    chunk
}

/// Read the sequence stamp a frame's chunk carries.
fn leading_sequence(chunk: &Bytes) -> Option<u32> {
    let field = std::str::from_utf8(chunk.get(..STAMP_BYTES)?).ok()?;
    field.parse().ok()
}

/// Flag a chunk that did not arrive at its expected position.
fn record_chunk(chunk: &Bytes, expected: &mut u32, disordered: &AtomicBool) {
    let sequence = leading_sequence(chunk).unwrap_or(u32::MAX);
    if sequence != *expected {
        disordered.store(true, Ordering::SeqCst);
    }
    *expected = sequence.saturating_add(1);
}

/// Handler that refuses to read until released, then counts every chunk.
///
/// `out_of_order` is set when a chunk arrives out of sequence, which is what a
/// body that entered discard mode looks like from the handler's side.
fn stalling_handler(
    chunks: Arc<AtomicUsize>,
    out_of_order: Arc<AtomicBool>,
    started: Arc<Notify>,
    release: Arc<Notify>,
) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, mut body: RequestBodyStream| {
        let counted = Arc::clone(&chunks);
        let disordered = Arc::clone(&out_of_order);
        let start_signal = Arc::clone(&started);
        let release_signal = Arc::clone(&release);
        Box::pin(async move {
            start_signal.notify_one();
            release_signal.notified().await;
            let mut expected = 0_u32;
            while let Some(chunk) = body.next().await {
                let received = chunk.ok()?;
                record_chunk(&received, &mut expected, &disordered);
                counted.fetch_add(1, Ordering::SeqCst);
            }
            Some(Bytes::from_static(b"done"))
        })
    })
}

/// Send the bulk body, report when the peer stops blocking, then read the reply.
///
/// The report is sent before the final frame, so a peer that is never stalled
/// signals while a stalled one cannot.
async fn send_bulk_body(
    mut client: Client,
    chunk: Vec<u8>,
    sent: oneshot::Sender<()>,
) -> TestResult<Vec<u8>> {
    for sequence in 1..=BULK_FRAMES {
        let frame = test_helpers::continuation_frame_payload(
            LIVE_KEY.into(),
            sequence.into(),
            &stamped(chunk.clone(), sequence),
            false,
        )?;
        send_envelope(&mut client, STREAM_ROUTE, &frame).await?;
    }
    // The observer may already have timed out and dropped its receiver, which
    // is exactly the saturated case this report marks, so the send's own
    // outcome cannot affect the test.
    let _report = sent.send(());
    let last = test_helpers::continuation_frame_payload(
        LIVE_KEY.into(),
        (BULK_FRAMES + 1).into(),
        &stamped(chunk, BULK_FRAMES + 1),
        true,
    )?;
    send_envelope(&mut client, STREAM_ROUTE, &last).await?;
    let reply = read_reply(&mut client).await?;
    close_client(&mut client).await?;
    Ok(reply)
}

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

/// A stalled handler stops the connection reading, so the peer blocks writing.
#[tokio::test]
async fn a_stalled_handler_stops_the_connection_reading_further_frames() -> TestResult {
    let chunks = Arc::new(AtomicUsize::new(0));
    let out_of_order = Arc::new(AtomicBool::new(false));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let server = serve_routes_within(
        AppLimits {
            frame_budget: PRESSURE_FRAME_BUDGET,
            budgets: Some(pressure_budgets()?),
        },
        vec![(
            STREAM_ROUTE,
            stalling_handler(
                Arc::clone(&chunks),
                Arc::clone(&out_of_order),
                Arc::clone(&started),
                Arc::clone(&release),
            ),
        )],
    )
    .await?;
    let mut client = connect(server.address).await?;

    let chunk = vec![b'x'; BULK_CHUNK];
    let first = test_helpers::first_frame_payload(
        LIVE_KEY.into(),
        &stamped(chunk.clone(), 0),
        false,
        None,
    )?;
    send_envelope(&mut client, STREAM_ROUTE, &first).await?;
    bounded("the stalled handler starting", started.notified()).await?;

    // The sender runs in its own task so the test never cancels a partially
    // written frame: it either finishes all thirty-two megabytes quickly,
    // proving there is no back-pressure, or stays blocked until the handler is
    // released.
    let (sent_tx, sent_rx) = oneshot::channel();
    let exchange = tokio::spawn(send_bulk_body(client, chunk, sent_tx));

    // A sender that finished, or that died because the connection aborted,
    // both mean the stalled handler did not hold the reader back.
    let outcome = timeout(PRESSURE_WAIT, sent_rx).await;
    assert_blocked(outcome.is_err(), &outcome);

    release.notify_one();
    let joined = bounded_for(DRAIN_WAIT, "the streaming exchange finishing", exchange).await?;
    let reply = joined??;

    assert_payload(
        &reply,
        b"done",
        "the handler should answer once its body drains",
    );
    assert_count(
        chunks.load(Ordering::SeqCst),
        TOTAL_CHUNKS,
        "every chunk held back by the stall should reach the handler once it resumes",
    );
    if out_of_order.load(Ordering::SeqCst) {
        return Err(
            "a chunk queued behind an admission must not be discarded or reordered"
                .to_owned()
                .into(),
        );
    }
    finish_server(server).await
}

/// A server shutdown ends a connection stalled on a chunk admission.
///
/// The loop cannot read while a chunk waits for budget, so the cancelled server
/// token is the one event that can still end the connection. Without that check
/// the admission is retried forever: the connection outlives the server's
/// drain, and [`finish_server`]'s bounded join is what fails.
#[tokio::test]
async fn a_server_shutdown_ends_a_connection_stalled_on_a_chunk_admission() -> TestResult {
    let chunks = Arc::new(AtomicUsize::new(0));
    let out_of_order = Arc::new(AtomicBool::new(false));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let server = serve_routes_within(
        AppLimits {
            frame_budget: PRESSURE_FRAME_BUDGET,
            budgets: Some(pressure_budgets()?),
        },
        vec![(
            STREAM_ROUTE,
            stalling_handler(
                Arc::clone(&chunks),
                Arc::clone(&out_of_order),
                Arc::clone(&started),
                Arc::clone(&release),
            ),
        )],
    )
    .await?;
    let mut client = connect(server.address).await?;

    let chunk = vec![b'x'; BULK_CHUNK];
    let first = test_helpers::first_frame_payload(
        LIVE_KEY.into(),
        &stamped(chunk.clone(), 0),
        false,
        None,
    )?;
    send_envelope(&mut client, STREAM_ROUTE, &first).await?;
    bounded("the stalled handler starting", started.notified()).await?;

    let (sent_tx, sent_rx) = oneshot::channel();
    let exchange = tokio::spawn(send_bulk_body(client, chunk, sent_tx));
    // A blocked peer proves the connection is stalled on an admission, so the
    // test cannot pass by shutting down a connection that was merely idle.
    let blocked = timeout(PRESSURE_WAIT, sent_rx).await.is_err();
    assert_blocked(blocked, &blocked);

    // The handler never reads and the peer never stops writing, so only the
    // shutdown signal can end this connection. The join inside `finish_server`
    // is bounded: a connection that retries its admission forever fails here.
    let finished = finish_server(server).await;
    exchange.abort();
    let _cancelled = exchange.await;
    finished
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
