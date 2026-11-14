//! Half-close delivery and post-cancellation shutdown.
//!
//! A peer that closes only its sending half still reads, so a response that
//! keeps producing frames must be delivered through a prepared connection. When
//! a producer instead stalls past the idle grace, the response is cancelled
//! without a terminator, so the connection must then stop rather than serve the
//! request buffered behind it.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/server_streaming_scenario.rs"]
mod scenario;
#[path = "common/server_streaming_io.rs"]
mod streaming_io;

use std::{sync::atomic::Ordering, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use scenario::{
    CONNECTION_SETTLE,
    FAST_ROUTE,
    MAX_FRAME_LENGTH,
    PACED_FRAME_COUNT,
    PACED_ROUTE,
    REQUEST_CORRELATION,
    STALLED_ROUTE,
    STATE_TIMEOUT,
    Scenario,
    ensure,
    finish,
    read_payloads,
    spawn_scenario,
};
use streaming_io::{join_producers, request_bytes, write_frame};
use tokio::{io::AsyncWriteExt, time::timeout};
use tokio_util::codec::Framed;
use wireframe::codec::{FrameCodec, LengthDelimitedFrameCodec};
use wireframe_testing::{TestError, TestResult};

/// The production idle grace, restated because the constant is crate-private.
///
/// Keep in step with `PEER_CLOSED_RESPONSE_GRACE` in `src/connection/drive.rs`.
const IDLE_GRACE: Duration = Duration::from_secs(30);
/// Slack allowed on either side of the idle grace.
///
/// The clock is paused, so the deadline is exact and this only absorbs
/// scheduling at the same virtual instant. It deliberately stays far below the
/// grace itself, so a longer deadline cannot satisfy the upper bound.
const GRACE_TOLERANCE: Duration = Duration::from_millis(1);
/// Latest instant at which the response must still be alive.
///
/// Stated as a literal rather than subtracting [`GRACE_TOLERANCE`] from
/// [`IDLE_GRACE`], because an unchecked `Duration` subtraction would panic.
/// Keep the pair one tolerance apart, so "alive before" and "ended by the
/// deadline" meet without leaving a gap a longer grace could hide in.
const GRACE_BEFORE_DEADLINE: Duration = Duration::from_millis(29_999);

/// A half-close must not truncate a response that keeps making progress.
///
/// The peer stops sending but keeps reading, and each producer pause is shorter
/// than the idle grace even though the whole response outlasts one. Delivery
/// therefore proves a valid half-close still works through a prepared
/// connection, because the deadline is renewed per frame.
#[tokio::test(start_paused = true)]
async fn half_close_still_delivers_a_progressing_stream() -> TestResult {
    let Scenario {
        client,
        connection,
        producers,
        ..
    } = spawn_scenario().await?;

    let (reader, mut writer) = tokio::io::split(client);
    write_frame(
        &mut writer,
        &request_bytes(PACED_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    // Close only the sending half: the peer still reads every response frame.
    writer.shutdown().await?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(reader, codec.decoder());
    let payloads = timeout(
        CONNECTION_SETTLE,
        read_payloads(&mut framed, PACED_FRAME_COUNT),
    )
    .await
    .map_err(|_| TestError::Msg("a half-close truncated a progressing stream".into()))??;
    ensure(
        payloads == vec![vec![1], vec![2], vec![3]],
        format!("every paced frame must be delivered in order: {payloads:?}"),
    )?;

    drop(framed);
    finish(connection).await?;
    join_producers(&producers).await
}

/// Wait *up to* `bound` for the next frame and require that none arrives.
///
/// The clock is paused, so an idle runtime advances virtual time only to the
/// next deadline. A bounded wait therefore ends at `bound` without reaching the
/// longer idle grace, which is what pins "still active immediately before the
/// deadline". A frame or end-of-file before it fails the check.
async fn ensure_no_frame_before<S>(frames: &mut S, bound: Duration, message: &str) -> TestResult
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    ensure(timeout(bound, frames.next()).await.is_err(), message)
}

/// Require the response to survive the idle grace and to end as it expires.
///
/// The window opens at an instant at which the actor has already armed its
/// deadline: the paused clock advances only when no task is ready, so the
/// connection task observes the closed write half, or the buffered request,
/// and arms the grace before this window can move. Checking immediately before
/// the deadline and then measuring the whole window therefore pins both sides
/// tightly: a *shorter* grace fails the first check, and a *later* deadline —
/// including a grace anchored after the window opened — fails the upper bound.
///
/// # Errors
///
/// Returns the transport failure reported by the frame reader.
async fn ensure_cancelled_at_the_grace<S>(frames: &mut S, message: &str) -> TestResult
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    let window_start = tokio::time::Instant::now();
    ensure_no_frame_before(frames, GRACE_BEFORE_DEADLINE, message).await?;
    expect_stream_end(frames).await?;

    let elapsed = window_start.elapsed();
    ensure(
        elapsed >= IDLE_GRACE,
        format!("the response must stay active for the whole idle grace: ended after {elapsed:?}"),
    )?;
    ensure(
        elapsed <= IDLE_GRACE + GRACE_TOLERANCE,
        format!("the response must end at the idle grace, not later: ended after {elapsed:?}"),
    )
}

/// Await end-of-file, letting the paused clock advance past the idle grace.
async fn expect_stream_end<S>(frames: &mut S) -> TestResult
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    let next = timeout(CONNECTION_SETTLE, frames.next())
        .await
        .map_err(|_| TestError::Msg("the cancelled connection stayed open".into()))?;
    ensure(
        next.is_none(),
        format!("no response may follow a cancelled stream: {next:?}"),
    )
}

/// A half-closed peer keeps a stalled response active until the grace expires.
///
/// The peer stops sending but keeps reading, so the response is not truncated
/// on EOF. It is bounded instead: still active immediately before the idle
/// grace, and cancelled once the deadline passes.
#[tokio::test(start_paused = true)]
async fn a_half_closed_peer_keeps_a_stalled_response_until_the_grace_expires() -> TestResult {
    let Scenario {
        client,
        connection,
        teardowns,
        producers,
    } = spawn_scenario().await?;

    let (reader, mut writer) = tokio::io::split(client);
    write_frame(
        &mut writer,
        &request_bytes(STALLED_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    writer.shutdown().await?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(reader, codec.decoder());
    let first = timeout(STATE_TIMEOUT, read_payloads(&mut framed, 1))
        .await
        .map_err(|_| TestError::Msg("the stalled stream produced no first frame".into()))??;
    ensure(
        first == vec![vec![1]],
        format!("the first stalled frame must be delivered: {first:?}"),
    )?;

    ensure_cancelled_at_the_grace(
        &mut framed,
        "a stalled response must stay active until the idle grace expires",
    )
    .await?;

    finish(connection).await?;
    ensure(
        teardowns.load(Ordering::SeqCst) == 1,
        "teardown runs once after the grace cancels the response",
    )?;
    join_producers(&producers).await
}

/// A cancelled response must end the connection before the buffered request.
///
/// The idle grace expires while a second request waits behind the stalled
/// stream. The cancelled stream has no terminator, so serving that request
/// would make its response indistinguishable from the truncated stream: the
/// connection closes instead and the client sees end-of-file.
#[tokio::test(start_paused = true)]
async fn a_cancelled_stream_ends_the_connection_before_the_buffered_request() -> TestResult {
    let Scenario {
        client,
        connection,
        teardowns,
        producers,
    } = spawn_scenario().await?;

    let (reader, mut writer) = tokio::io::split(client);
    write_frame(
        &mut writer,
        &request_bytes(STALLED_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(reader, codec.decoder());
    // The first frame arrives immediately; the producer then stalls for longer
    // than the idle grace.
    let first = timeout(STATE_TIMEOUT, read_payloads(&mut framed, 1))
        .await
        .map_err(|_| TestError::Msg("the stalled stream produced no first frame".into()))??;
    ensure(
        first == vec![vec![1]],
        format!("the first stalled frame must be delivered: {first:?}"),
    )?;

    // The fast request is buffered behind the stalled response, so the read
    // half stops being polled and the response is bounded by the idle grace.
    write_frame(
        &mut writer,
        &request_bytes(FAST_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;

    // The buffered request is what stops the countdown from being anchored in
    // the future: the connection task reads it and arms the grace before the
    // paused clock can advance past this point.
    ensure_cancelled_at_the_grace(
        &mut framed,
        "a buffered request must not cancel the response before the grace expires",
    )
    .await?;

    finish(connection).await?;
    ensure(
        teardowns.load(Ordering::SeqCst) == 1,
        "teardown runs once after a cancelled stream closes the connection",
    )?;
    join_producers(&producers).await
}
