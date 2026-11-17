//! Unit tests for the non-blocking read attempt and the idle expiry.

use std::num::NonZeroUsize;

use futures::stream;
use tokio::time::{Duration, Instant, advance};

use super::*;
use crate::message_assembler::{EnvelopeRouting, FirstFrameHeader, FirstFrameInput, MessageKey};

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

/// Drive one read wait that a push batch cancels after 100 ms.
async fn wait_cancelled_by_a_push<T, R>(inbound: &mut InboundFrames<T>, reader: &mut R)
where
    R: Stream<Item = io::Result<T>> + Unpin,
{
    let attempt = tokio::time::timeout(
        Duration::from_millis(100),
        inbound.next(reader, 400, &CancellationToken::new()),
    )
    .await;
    assert!(attempt.is_err(), "the reader stays silent throughout");
}

/// Reader state holding one incomplete assembly that any purge evicts.
///
/// The assembly timeout is zero so that a purge always drops it; the
/// assertions then measure *when* the purge runs rather than the assembly
/// clock, which is a `std::time::Instant` that a paused runtime does not
/// advance.
fn inbound_with_incomplete_assembly() -> InboundFrames<u8> {
    let mut assembly = MessageAssemblyState::new(
        NonZeroUsize::new(1024).unwrap_or(NonZeroUsize::MIN),
        Duration::ZERO,
    );
    let first = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: 4,
        total_body_len: Some(8),
        is_last: false,
    };
    let input = FirstFrameInput::new(&first, EnvelopeRouting::default(), Vec::new(), b"half")
        .expect("the incomplete frame is valid");
    assembly
        .accept_first_frame(input)
        .expect("the first frame is buffered");
    InboundFrames {
        pending: None,
        pipeline: FramePipeline::new(None),
        message_assembly: Some(assembly),
        read_deadline: None,
    }
}

/// Number of incomplete assemblies still buffered.
fn buffered(inbound: &InboundFrames<u8>) -> Option<usize> {
    inbound
        .message_assembly
        .as_ref()
        .map(MessageAssemblyState::buffered_count)
}

/// A client that only receives pushes must still expire its buffered state.
///
/// Each push batch cancels the pending read before its timeout fires. A
/// deadline re-armed inside every wait would therefore restart with each
/// batch, so the expiry purge would never run on a connection that only
/// receives pushes. The deadline belongs to the reader, so it is measured
/// across the cancelled waits: nothing is purged before it passes, and the
/// purge runs once it does.
#[tokio::test(start_paused = true)]
async fn pushes_do_not_postpone_the_inbound_expiry() {
    let mut inbound = inbound_with_incomplete_assembly();
    let mut reader = stream::pending::<io::Result<u8>>();

    // Three cancelled waits keep the 400 ms read deadline in the future.
    for _ in 0..3 {
        wait_cancelled_by_a_push(&mut inbound, &mut reader).await;
    }
    assert_eq!(
        buffered(&inbound),
        Some(1),
        "no purge may run before the read deadline passes"
    );

    // Two more carry the elapsed time past the deadline; the next wait
    // must purge even though a push cancels it again.
    for _ in 0..3 {
        wait_cancelled_by_a_push(&mut inbound, &mut reader).await;
    }
    assert_eq!(
        buffered(&inbound),
        Some(0),
        "the incomplete assembly must expire despite the pushes"
    );
}

/// A push lane that is always ready must not defer the expiry either.
///
/// When a batch is ready on every pass the biased select serves pushes, so
/// the read wait is never polled and only the probe runs. A deadline armed
/// solely by the wait would therefore never start, and the buffered
/// assembly would outlive the timeout it was given. The probe arms the
/// deadline, so the purge runs from it without the read ever being driven.
#[tokio::test(start_paused = true)]
async fn a_ready_push_lane_does_not_postpone_the_inbound_expiry() {
    let mut inbound = inbound_with_incomplete_assembly();
    let mut reader = stream::pending::<io::Result<u8>>();

    // Arming the deadline is not yet expiry: the assembly survives the
    // first probe and the rest of the interval.
    assert!(matches!(
        inbound.try_next(&mut reader, 400).await,
        Ok(NextFrame::TimedOut)
    ));
    advance(Duration::from_millis(399)).await;
    assert!(matches!(
        inbound.try_next(&mut reader, 400).await,
        Ok(NextFrame::TimedOut)
    ));
    assert_eq!(
        buffered(&inbound),
        Some(1),
        "no purge may run before the armed deadline passes"
    );

    advance(Duration::from_millis(2)).await;
    assert!(matches!(
        inbound.try_next(&mut reader, 400).await,
        Ok(NextFrame::TimedOut)
    ));
    assert_eq!(
        buffered(&inbound),
        Some(0),
        "the probe must purge once the armed deadline has passed"
    );
}
