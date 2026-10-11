//! Driving one actor-drain workload.
//!
//! The rate limiter is disabled throughout, because the issue asks that
//! rate-limiter timing not dominate an ownership measurement: a token bucket
//! would pace the preload and swamp the loop's own cost.

use std::{
    hint::black_box,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;
use wireframe::{
    app::Envelope,
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushHandle, PushQueues},
};

use super::workload::{ActorMeasurement, ActorWorkload, CancellationMode};

/// Byte stored in every frame payload so emitted frames stay distinguishable.
const PAYLOAD_BYTE: u8 = 0xab;

/// Payload length for every frame, kept non-zero so payload buffers allocate.
const PAYLOAD_LEN: usize = 64;

/// Build queues sized for `workload` with rate limiting disabled.
fn build_queues(
    workload: ActorWorkload,
) -> Result<(PushQueues<Envelope>, PushHandle<Envelope>), String> {
    PushQueues::<Envelope>::builder()
        .high_capacity(workload.queue_capacity)
        .low_capacity(workload.queue_capacity)
        .unlimited()
        .build()
        .map_err(|err| format!("failed to build push queues: {err}"))
}

/// Build one distinguishable frame.
///
/// The route cycles over eight values via a mask rather than `%`, because
/// `clippy::integer-division-remainder-used` is denied workspace-wide. The
/// route id is not asserted anywhere; it exists so that frames are not
/// byte-identical to one another.
fn frame(id: usize) -> Envelope {
    let route = u32::try_from(id & 0x7).unwrap_or(u32::MAX);
    Envelope::new(route, None, vec![PAYLOAD_BYTE; PAYLOAD_LEN])
}

/// Preload every frame, the high queue first.
///
/// The lanes are filled in priority order so the queue the actor polls first
/// also holds the frames the label describes as "high".
async fn preload(workload: ActorWorkload, handle: &PushHandle<Envelope>) -> Result<(), String> {
    for id in 0..workload.high_frames() {
        handle
            .push_high_priority(frame(id))
            .await
            .map_err(|err| format!("high-priority preload failed: {err}"))?;
    }
    for id in 0..workload.low_frames() {
        handle
            .push_low_priority(frame(id))
            .await
            .map_err(|err| format!("low-priority preload failed: {err}"))?;
    }
    Ok(())
}

/// Build the hooks a workload needs, cancelling mid-drain when asked.
///
/// The cancellation rides on `before_send` rather than a timer or a spawned
/// task. A timer would make the regime non-deterministic; a spawned task cannot
/// run at all while the actor holds the thread, because each iteration drives
/// the loop to completion before yielding. Cancelling from the output hook is
/// the only point that is both inside the loop and fully deterministic.
fn workload_hooks(
    workload: ActorWorkload,
    shutdown: &CancellationToken,
    cancelled_during_drain: &Arc<AtomicBool>,
) -> ProtocolHooks<Envelope, wireframe::NoProtocolError> {
    let mut hooks = ProtocolHooks::default();
    if !matches!(
        workload.cancellation,
        CancellationMode::CancelledDuringDrain
    ) {
        return hooks;
    }

    let token = shutdown.clone();
    let observed = Arc::clone(cancelled_during_drain);
    hooks.before_send = Some(Box::new(
        move |_frame: &mut Envelope, _ctx: &mut ConnectionContext| {
            // Cancelling on every frame after the first is harmless: the token is
            // idempotent, and re-asserting it keeps the request visible even if the
            // loop drains the entire queue before the select loop re-evaluates.
            token.cancel();
            observed.store(true, Ordering::SeqCst);
        },
    ));
    hooks
}

/// Drive one iteration to completion and report how many frames it emitted.
async fn drain_once(
    workload: ActorWorkload,
    cancelled_during_drain: &Arc<AtomicBool>,
) -> Result<usize, String> {
    let shutdown = CancellationToken::new();
    let hooks = workload_hooks(workload, &shutdown, cancelled_during_drain);
    let (queues, handle) = build_queues(workload)?;
    let producer = handle.clone();
    preload(workload, &producer).await?;
    // Dropping the only producer before the run closes the queues once every
    // buffered frame has been read, which is how the uncancelled regime ends.
    drop(producer);

    if matches!(workload.cancellation, CancellationMode::CancelledBeforeRun) {
        shutdown.cancel();
    }

    let channels = ConnectionChannels::new(queues, handle);
    let mut actor = ConnectionActor::with_hooks(channels, None, shutdown, hooks);
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|err| format!("actor run failed: {err:?}"))?;
    Ok(black_box(out.len()))
}

/// Drive `iterations` actor drains and report the last one's frame count.
///
/// # Errors
///
/// Returns a description of the invalid workload, queue-construction failure, or
/// actor failure. A workload whose frames do not fit in its queues is rejected,
/// because measuring it would report a different regime than its label claims.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::runtime_ownership_benchmarks::{
///     ActorWorkload,
///     CancellationMode,
///     DEFAULT_ACTOR_QUEUE_CAPACITY,
///     DEFAULT_FRAME_COUNT,
///     measure_actor_drain,
/// };
///
/// # async fn example() -> Result<(), String> {
/// let measurement = measure_actor_drain(
///     ActorWorkload {
///         frames: DEFAULT_FRAME_COUNT,
///         queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
///         cancellation: CancellationMode::Disabled,
///     },
///     1,
/// )
/// .await?;
/// assert_eq!(measurement.frames_emitted, DEFAULT_FRAME_COUNT);
/// # Ok(())
/// # }
/// ```
pub async fn measure_actor_drain(
    workload: ActorWorkload,
    iterations: u64,
) -> Result<ActorMeasurement, String> {
    workload.validate()?;
    if iterations == 0 {
        return Ok(ActorMeasurement {
            operations: 0,
            frames_emitted: 0,
            elapsed: Duration::ZERO,
            cancelled_during_drain: false,
        });
    }

    let cancelled_during_drain = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let mut frames_emitted = 0usize;
    for _ in 0..iterations {
        frames_emitted = drain_once(workload, &cancelled_during_drain).await?;
    }
    let elapsed = started.elapsed();

    Ok(ActorMeasurement {
        operations: iterations,
        frames_emitted,
        elapsed,
        cancelled_during_drain: cancelled_during_drain.load(Ordering::SeqCst),
    })
}
