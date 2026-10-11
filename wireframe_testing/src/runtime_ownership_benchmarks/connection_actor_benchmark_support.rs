//! Workloads and measurement for the connection actor's event loop.
//!
//! The actor loop is the hot path Epic #635 proposes to re-topologize, so its
//! baseline must describe how many queued frames actually travelled through the
//! loop, under which cancellation regime, and with what queue capacities. Those
//! three facts are workload parameters here rather than constants baked into a
//! benchmark binary, so the same scenario keeps working after the refactor and
//! can be pointed at an old and a new implementation unchanged.
//!
//! Frames are [`Envelope`] values rather than the crate's test-only `u8` frame.
//! `Envelope` implements [`Packet`](wireframe::app::Packet) without any feature
//! gate, so this helper works from a plain `wireframe` dependency and exercises
//! the same frame type production connections carry.
//!
//! # Cancellation regimes
//!
//! [`CancellationMode`] distinguishes the three behaviours the issue asks for.
//! They are not three timings of one scenario; they are three different control
//! flows through the loop, and the frame counts they produce differ by design:
//!
//! - [`CancellationMode::Disabled`] never cancels. The actor terminates because every producer has
//!   dropped, so the queues report closure.
//! - [`CancellationMode::CancelledBeforeRun`] cancels before [`run`] is called. The actor aborts
//!   immediately and emits nothing.
//! - [`CancellationMode::CancelledDuringDrain`] preloads every frame, then cancels from inside the
//!   output hook on the first emitted frame. Shutdown is therefore observed *while* the loop is
//!   draining, which is the regime that proves the loop neither short-circuits on shutdown nor
//!   abandons frames already buffered in the high- and low-priority queues.
//!
//! [`run`]: wireframe::connection::ConnectionActor::run
//!
//! # Preconditions
//!
//! Every frame a workload names must fit in the queues, so
//! `frames <= 2 * queue_capacity`. A workload that violates this is rejected by
//! [`measure_actor_drain`] rather than measured, because an over-capacity
//! workload would make the preload block on back-pressure and silently change
//! which regime was measured.

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
    push::PushQueues,
};

/// Frame count used by the benchmark's default actor workload.
pub const DEFAULT_FRAME_COUNT: usize = 1024;

/// Per-queue capacity used by the benchmark's default actor workload.
///
/// At least half [`DEFAULT_FRAME_COUNT`], so the default workload satisfies the
/// fit-in-queues precondition without relying on back-pressure.
pub const DEFAULT_QUEUE_CAPACITY: usize = 512;

/// Byte stored in every frame payload so emitted frames stay distinguishable.
const PAYLOAD_BYTE: u8 = 0xab;

/// Payload length for every frame, kept non-zero so payload buffers allocate.
const PAYLOAD_LEN: usize = 64;

/// How a workload treats the actor's shutdown token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancellationMode {
    /// Never cancel; the actor ends when its queues close.
    Disabled,
    /// Cancel before the loop starts, so the actor aborts without draining.
    CancelledBeforeRun,
    /// Cancel from the output hook on the first frame, mid-drain.
    CancelledDuringDrain,
}

impl CancellationMode {
    /// Short identifier used in benchmark labels.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::CancellationMode;
    ///
    /// assert_eq!(CancellationMode::Disabled.label(), "cancel_disabled");
    /// ```
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "cancel_disabled",
            Self::CancelledBeforeRun => "cancel_before_run",
            Self::CancelledDuringDrain => "cancel_during_drain",
        }
    }
}

/// One actor-drain scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorWorkload {
    /// Total frames preloaded across both queues.
    pub frames: usize,
    /// Capacity of each of the high- and low-priority queues.
    pub queue_capacity: usize,
    /// Cancellation regime to exercise.
    pub cancellation: CancellationMode,
}

impl Default for ActorWorkload {
    fn default() -> Self {
        Self {
            frames: DEFAULT_FRAME_COUNT,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            cancellation: CancellationMode::Disabled,
        }
    }
}

impl ActorWorkload {
    /// Frames placed on the high-priority queue.
    ///
    /// The high queue takes the larger half so an odd frame count still fills
    /// both queues, keeping "both priorities" genuinely exercised.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{ActorWorkload, CancellationMode};
    ///
    /// let workload = ActorWorkload {
    ///     frames: 17,
    ///     queue_capacity: 512,
    ///     cancellation: CancellationMode::Disabled,
    /// };
    /// assert_eq!((workload.high_frames(), workload.low_frames()), (9, 8));
    /// ```
    #[must_use]
    pub const fn high_frames(self) -> usize { self.frames.div_ceil(2) }

    /// Frames placed on the low-priority queue.
    #[must_use]
    pub const fn low_frames(self) -> usize { self.frames - self.high_frames() }

    /// Benchmark label embedding every recorded parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{ActorWorkload, CancellationMode};
    ///
    /// let workload = ActorWorkload {
    ///     frames: 256,
    ///     queue_capacity: 512,
    ///     cancellation: CancellationMode::Disabled,
    /// };
    /// assert_eq!(workload.label(), "frames_256_capacity_512_cancel_disabled");
    /// ```
    #[must_use]
    pub fn label(self) -> String {
        format!(
            "frames_{}_capacity_{}_{}",
            self.frames,
            self.queue_capacity,
            self.cancellation.label()
        )
    }

    /// Number of frames this workload must emit.
    ///
    /// The abort path emits nothing; every other regime emits every preloaded
    /// frame, including the one which observes shutdown mid-drain.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{ActorWorkload, CancellationMode};
    ///
    /// let mut workload = ActorWorkload {
    ///     frames: 17,
    ///     queue_capacity: 512,
    ///     cancellation: CancellationMode::Disabled,
    /// };
    /// assert_eq!(workload.expected_frames(), 17);
    /// workload.cancellation = CancellationMode::CancelledBeforeRun;
    /// assert_eq!(workload.expected_frames(), 0);
    /// ```
    #[must_use]
    pub const fn expected_frames(self) -> usize {
        match self.cancellation {
            CancellationMode::CancelledBeforeRun => 0,
            CancellationMode::Disabled | CancellationMode::CancelledDuringDrain => self.frames,
        }
    }

    /// Reject workloads the harness cannot measure as described.
    fn validate(self) -> Result<(), String> {
        if self.queue_capacity == 0 {
            return Err("queue_capacity must be at least 1".to_string());
        }
        let headroom = self
            .queue_capacity
            .checked_mul(2)
            .ok_or_else(|| "queue_capacity is too large to double".to_string())?;
        if self.frames > headroom {
            return Err(format!(
                "frames {} exceeds total queue capacity {}; preloading would block on \
                 back-pressure and change the measured regime",
                self.frames, headroom
            ));
        }
        Ok(())
    }
}

/// Result of driving the actor for one measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorMeasurement {
    /// Workload iterations completed.
    pub operations: u64,
    /// Frames emitted by the final iteration.
    pub frames_emitted: usize,
    /// Wall-clock time across every iteration.
    pub elapsed: Duration,
    /// Whether the output hook observed a cancellation request mid-drain.
    ///
    /// Recorded so that a test can tell "cancellation was requested during the
    /// drain" from "the hook silently never fired", which would otherwise make
    /// [`CancellationMode::CancelledDuringDrain`] indistinguishable from
    /// [`CancellationMode::Disabled`].
    pub cancelled_during_drain: bool,
}

/// Build queues sized for `workload` with rate limiting disabled.
///
/// The limiter is disabled because the issue asks that rate-limiter timing not
/// dominate an ownership measurement; a token bucket would otherwise pace the
/// preload and swamp the loop's own cost.
fn build_queues(
    workload: ActorWorkload,
) -> Result<(PushQueues<Envelope>, wireframe::push::PushHandle<Envelope>), String> {
    PushQueues::<Envelope>::builder()
        .high_capacity(workload.queue_capacity)
        .low_capacity(workload.queue_capacity)
        .unlimited()
        .build()
        .map_err(|err| format!("failed to build push queues: {err}"))
}

/// Preload every frame, the high queue first.
///
/// The lanes are filled in priority order so the queue the actor polls first
/// also holds the frames the label describes as "high".
async fn preload(
    workload: ActorWorkload,
    handle: &wireframe::push::PushHandle<Envelope>,
) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    //! Unit coverage for workload arithmetic and rejection of unfit workloads.

    use super::{ActorWorkload, CancellationMode, measure_actor_drain};

    #[test]
    fn split_covers_every_frame_across_both_queues() {
        for frames in [1usize, 2, 17, 256] {
            let workload = ActorWorkload {
                frames,
                queue_capacity: 512,
                cancellation: CancellationMode::Disabled,
            };
            assert_eq!(
                workload.high_frames() + workload.low_frames(),
                frames,
                "the two queues must hold every frame for frames={frames}"
            );
        }
    }

    #[tokio::test]
    async fn over_capacity_workload_is_rejected() {
        let error = measure_actor_drain(
            ActorWorkload {
                frames: 3,
                queue_capacity: 1,
                cancellation: CancellationMode::Disabled,
            },
            1,
        )
        .await
        .expect_err("a workload that cannot fit its queues must be rejected");

        assert!(
            error.contains("exceeds total queue capacity"),
            "unexpected rejection reason: {error}"
        );
    }

    #[tokio::test]
    async fn zero_iterations_measures_nothing() {
        let measurement = measure_actor_drain(ActorWorkload::default(), 0)
            .await
            .expect("zero iterations must succeed");

        assert_eq!(measurement.operations, 0);
        assert_eq!(measurement.frames_emitted, 0);
    }
}
