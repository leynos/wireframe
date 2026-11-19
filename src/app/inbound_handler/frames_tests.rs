//! Unit tests for the non-blocking read attempt and the idle expiry.

use bytes::Bytes;
use futures::stream;
use tokio::time::{Duration, Instant, advance};

use super::{
    super::push_service::{StreamStep, StreamWait, next_stream_step},
    *,
};
use crate::{
    app::Envelope,
    codec::LengthDelimitedFrameCodec,
    connection::{ConnectionActor, ConnectionChannels, UntrackedActorInit},
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushHandle, PushQueues},
};

/// Read timeout the expiry tests arm.
const READ_TIMEOUT_MS: u64 = 400;

/// Capacity of each push lane the scheduler test delivers from.
const LANE_CAPACITY: usize = 4;

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
    let mut inbound = incomplete_inbound::<u8>();
    let mut reader = stream::pending::<io::Result<u8>>();

    // Three cancelled waits keep the 400 ms read deadline in the future.
    for _ in 0..3 {
        wait_cancelled_by_a_push(&mut inbound, &mut reader).await;
    }
    assert_eq!(
        buffered_assemblies(&inbound),
        Some(1),
        "no purge may run before the read deadline passes"
    );

    // Two more carry the elapsed time past the deadline; the next wait
    // must purge even though a push cancels it again.
    for _ in 0..3 {
        wait_cancelled_by_a_push(&mut inbound, &mut reader).await;
    }
    assert_eq!(
        buffered_assemblies(&inbound),
        Some(0),
        "the incomplete assembly must expire despite the pushes"
    );
}

/// Actor with an open push lane the scheduler test can take from.
///
/// The lane is the competitor the select must not let starve the expiry, so it
/// is the actor's own queue rather than a stand-in.
fn push_actor(
    shutdown: &CancellationToken,
) -> (ConnectionActor<Envelope, (), Bytes>, PushHandle<Bytes>) {
    let (queues, handle) = PushQueues::<Bytes>::builder()
        .high_capacity(LANE_CAPACITY)
        .low_capacity(LANE_CAPACITY)
        .build()
        .expect("the test lane capacities are valid");
    let actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle.clone()),
        response: None,
        shutdown: shutdown.clone(),
        hooks: ProtocolHooks::default(),
        ctx: ConnectionContext::default(),
    });
    (actor, handle)
}

/// Take one scheduled step with a freshly queued push waiting.
///
/// Each call consumes the batch it wins, so the lane is replenished first; the
/// returned step must be the pushes, which is what proves the ready lane beat
/// the read on this pass.
async fn step_with_ready_push<R>(
    actor: &mut ConnectionActor<Envelope, (), Bytes>,
    handle: &PushHandle<Bytes>,
    inbound: &mut InboundFrames<Bytes>,
    reader: &mut R,
    shutdown: &CancellationToken,
) where
    R: Stream<Item = io::Result<Bytes>> + Unpin,
{
    handle
        .push_high_priority(Bytes::from_static(b"queued"))
        .await
        .expect("the lane has room for the next frame");
    let step = next_stream_step::<LengthDelimitedFrameCodec, _>(
        actor,
        StreamWait {
            inbound,
            reader,
            read_timeout_ms: READ_TIMEOUT_MS,
            shutdown,
            read_first: false,
        },
    )
    .await
    .expect("the wait must not fail");
    assert!(
        matches!(step, StreamStep::Pushes(_)),
        "the ready push lane must win the pass"
    );
}

/// A push lane that is always ready must not defer the inbound expiry.
///
/// When a batch is ready on every pass the biased select serves pushes, so the
/// read wait is cancelled before its timeout fires. A deadline armed solely by
/// the wait would therefore never start, and the buffered assembly would
/// outlive the timeout it was given. `next_stream_step` runs the expiry command
/// before it selects, so the purge happens on the scheduler path even though
/// the read is never driven.
#[tokio::test(start_paused = true)]
async fn a_ready_push_lane_does_not_postpone_the_inbound_expiry() {
    let shutdown = CancellationToken::new();
    let (mut actor, handle) = push_actor(&shutdown);
    let mut inbound = incomplete_inbound::<Bytes>();
    let mut reader = stream::pending::<io::Result<Bytes>>();

    // Arming the deadline is not yet expiry: the assembly survives the first
    // pass and the rest of the interval.
    step_with_ready_push(&mut actor, &handle, &mut inbound, &mut reader, &shutdown).await;
    advance(Duration::from_millis(READ_TIMEOUT_MS - 1)).await;
    step_with_ready_push(&mut actor, &handle, &mut inbound, &mut reader, &shutdown).await;
    assert_eq!(
        buffered_assemblies(&inbound),
        Some(1),
        "no purge may run before the armed deadline passes"
    );

    advance(Duration::from_millis(2)).await;
    step_with_ready_push(&mut actor, &handle, &mut inbound, &mut reader, &shutdown).await;
    assert_eq!(
        buffered_assemblies(&inbound),
        Some(0),
        "the expiry command must purge once the deadline has passed"
    );
}

/// A buffered frame must not take the turn a ready push batch owns.
///
/// The read arm returns a buffered frame without waiting, so probing for one on
/// every pass would hand the reader every turn while an accepted push waited
/// behind it. The buffered frame keeps its place and is served on the turn
/// after the batch.
#[tokio::test(start_paused = true)]
async fn a_buffered_frame_does_not_starve_a_ready_push() {
    use crate::app::codec_driver::FramePipeline;

    let shutdown = CancellationToken::new();
    let (mut actor, handle) = push_actor(&shutdown);
    let buffered = Bytes::from_static(b"buffered");
    let mut inbound = InboundFrames::<Bytes> {
        pending: Some(buffered.clone()),
        pipeline: FramePipeline::new(None),
        message_assembly: None,
        read_deadline: None,
    };
    let mut reader = stream::pending::<io::Result<Bytes>>();
    handle
        .push_high_priority(Bytes::from_static(b"queued"))
        .await
        .expect("the lane has room for one frame");

    let step = next_stream_step::<LengthDelimitedFrameCodec, _>(
        &mut actor,
        StreamWait {
            inbound: &mut inbound,
            reader: &mut reader,
            read_timeout_ms: READ_TIMEOUT_MS,
            shutdown: &shutdown,
            read_first: false,
        },
    )
    .await
    .expect("the wait must not fail");

    assert!(
        matches!(step, StreamStep::Pushes(_)),
        "the ready push batch must win the pass"
    );
    assert_eq!(
        inbound.pending,
        Some(buffered),
        "the buffered frame must keep its place for the next turn"
    );
}

/// The read probe must never purge as a side effect of being called.
///
/// Expiry is a command the loop runs on each pass, so a probe stays a query:
/// a caller that polls the reader without expiring first keeps its buffered
/// state even after the deadline. Pinning that keeps the two responsibilities
/// separate, and keeps the loop's explicit call the only thing that drops it.
#[tokio::test(start_paused = true)]
async fn the_read_probe_never_purges_buffered_state() {
    let mut inbound = incomplete_inbound::<u8>();
    let mut reader = stream::pending::<io::Result<u8>>();

    // Arm the deadline, then let it pass without running the command again.
    inbound.expire_silence(400);
    advance(Duration::from_millis(401)).await;
    assert!(matches!(
        inbound.try_next(&mut reader).await,
        Ok(NextFrame::TimedOut)
    ));
    assert_eq!(
        buffered_assemblies(&inbound),
        Some(1),
        "the probe must not purge; only the expiry command may"
    );

    inbound.expire_silence(400);
    assert_eq!(
        buffered_assemblies(&inbound),
        Some(0),
        "the same interval expires once the command runs"
    );
}
