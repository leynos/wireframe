//! Unit tests for the non-blocking read attempt and the idle expiry.

use futures::stream;
use tokio::time::{Duration, Instant, advance};

use super::*;
use crate::message_assembler::{
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    MessageAssemblyState,
    MessageKey,
    StreamingAssemblyId,
};

/// Read timeout the expiry tests arm.
const READ_TIMEOUT_MS: u64 = 400;

/// The post-batch read probe must not wait for a timer tick.
///
/// A pending reader reported as silence at once, because the push lanes
/// are polled again immediately rather than after up to a millisecond.
#[tokio::test(start_paused = true)]
async fn the_read_probe_spends_no_time_on_a_pending_reader() -> io::Result<()> {
    let mut reader = stream::pending::<io::Result<u8>>();
    let mut pending: Option<u8> = None;
    let started = Instant::now();

    let next = poll_frame(&mut pending, &mut reader, Duration::ZERO).await?;

    if !matches!(next, NextFrame::TimedOut) {
        return Err(io::Error::other("a pending read reports silence"));
    }
    if started.elapsed() != Duration::ZERO {
        return Err(io::Error::other(
            "the probe must poll the reader once rather than arm a timer",
        ));
    }
    Ok(())
}

/// A ready frame still wins over the push lanes, and a buffered frame is
/// taken first without polling the reader again.
#[tokio::test]
async fn the_read_probe_returns_a_ready_frame() -> io::Result<()> {
    let mut reader = stream::iter([Ok(1_u8)]);
    let mut pending = Some(2_u8);

    let next = poll_frame(&mut pending, &mut reader, Duration::ZERO).await?;

    if !matches!(next, NextFrame::Ready(2)) {
        return Err(io::Error::other(
            "a buffered frame must be served before the reader is polled",
        ));
    }
    let ready = poll_frame(&mut pending, &mut reader, Duration::ZERO).await?;
    if !matches!(ready, NextFrame::Ready(1)) {
        return Err(io::Error::other("the ready reader frame must be returned"));
    }
    Ok(())
}

/// Arm the read deadline and let it pass without running the command.
async fn let_the_read_deadline_pass(inbound: &mut InboundFrames<u8, u8>) {
    let mut evicted = Vec::new();
    InboundExpiry::new(
        &mut inbound.message_assembly,
        &mut inbound.read_deadline,
        READ_TIMEOUT_MS,
        &mut evicted,
    )
    .due();
    advance(Duration::from_millis(READ_TIMEOUT_MS + 1)).await;
}

/// Run the expiry command once and return the streaming bodies it evicted.
fn run_expiry(inbound: &mut InboundFrames<u8, u8>, evicted: &mut Vec<StreamingAssemblyId>) {
    InboundExpiry::new(
        &mut inbound.message_assembly,
        &mut inbound.read_deadline,
        READ_TIMEOUT_MS,
        evicted,
    )
    .expire_silence();
}

/// The read probe must never purge as a side effect of being called.
///
/// Expiry is a command the loop runs on each pass, so a probe stays a query:
/// a caller that polls the reader without expiring first keeps its buffered
/// state even after the deadline. Pinning that keeps the two responsibilities
/// separate, and keeps the expiry command the only thing that drops it.
#[tokio::test(start_paused = true)]
async fn the_read_probe_never_purges_buffered_state() -> io::Result<()> {
    let mut inbound = incomplete_inbound::<u8>();
    let mut reader = stream::pending::<io::Result<u8>>();

    let_the_read_deadline_pass(&mut inbound).await;
    if !matches!(inbound.try_next(&mut reader).await?, NextFrame::TimedOut) {
        return Err(io::Error::other("the reader stays silent throughout"));
    }
    // Reported rather than asserted: this test returns a `Result`, so a
    // failure belongs in the error it returns.
    if buffered_assemblies(&inbound) != Some(1) {
        return Err(io::Error::other(
            "the probe must not purge; only the expiry command may",
        ));
    }

    run_expiry(&mut inbound, &mut Vec::new());
    if buffered_assemblies(&inbound) != Some(0) {
        return Err(io::Error::other(
            "the same interval expires once the command runs",
        ));
    }
    Ok(())
}

/// A streaming assembly evicted by the expiry must be reported to the loop.
///
/// A push write runs this command while it holds the reader, so it cannot
/// answer the handler itself. Dropping the identifier instead would leave the
/// handler waiting for chunks that can never arrive, pinning the connection's
/// only streaming slot for the rest of the connection.
#[tokio::test(start_paused = true)]
async fn the_expiry_reports_the_streaming_bodies_it_evicts() {
    let mut inbound = streaming_inbound();
    let mut evicted = Vec::new();

    let_the_read_deadline_pass(&mut inbound).await;
    run_expiry(&mut inbound, &mut evicted);

    assert_eq!(
        evicted.len(),
        1,
        "the evicted streaming body must be reported once"
    );

    // The assembly is gone, so a later pass reports nothing to fail twice.
    run_expiry(&mut inbound, &mut evicted);
    assert_eq!(evicted.len(), 1, "an evicted body is reported only once");
}

/// Inbound state holding one incomplete streaming assembly a purge evicts.
///
/// The assembly timeout is zero so that the purge always drops it, matching
/// [`incomplete_inbound`].
fn streaming_inbound() -> InboundFrames<u8, u8> {
    let mut assembly = MessageAssemblyState::new(
        std::num::NonZeroUsize::new(1024).unwrap_or(std::num::NonZeroUsize::MIN),
        Duration::ZERO,
    );
    let first = FirstFrameHeader {
        message_key: MessageKey(2),
        metadata_len: 0,
        body_len: 4,
        total_body_len: Some(8),
        is_last: false,
    };
    let input = FirstFrameInput::new(&first, EnvelopeRouting::default(), Vec::new(), b"half")
        .expect("the incomplete frame is valid");
    assembly
        .accept_streaming_first_frame(input)
        .expect("the streaming first frame is accepted");
    InboundFrames {
        pending: None,
        pipeline: FramePipeline::new(None),
        message_assembly: Some(assembly),
        read_deadline: None,
    }
}
