//! Cloneable handle used by producers to push frames to a connection.

use std::{
    future::{Future, poll_fn},
    time::Duration,
};

use leaky_bucket::RateLimiter;
use log::{debug, warn};
use tokio::{sync::mpsc, time::sleep};

#[cfg(loom)]
use super::probe::PushHandleProbe;
use super::{
    FrameLike,
    FrameLimits,
    PushError,
    PushPolicy,
    PushPriority,
    dlq::DeadLetterState,
    sync::{Arc, Weak},
    waits::{PushWaitCounters, PushWaitCounts, record_first_pending},
};

/// Shared state for [`PushHandle`].
///
/// Holds the high- and low-priority channels alongside an optional rate
/// limiter and the dead-letter accounting used when pushes are discarded.
///
/// - `high_prio_tx` – channel for frames that must be sent before any low-priority traffic.
/// - `low_prio_tx` – channel for best-effort frames.
/// - `limiter` – optional rate-limiter enforcing global push throughput.
/// - `dead_letter` – drop destination and diagnostics for discarded frames.
/// - `waits` – what this handle's awaited pushes had to wait for.
pub(crate) struct PushHandleInner<F> {
    /// Sender for urgent frames, preserving priority at the actor boundary.
    pub(crate) high_prio_tx: mpsc::Sender<F>,
    /// Sender for best-effort frames.
    pub(crate) low_prio_tx: mpsc::Sender<F>,
    /// Shared limiter that bounds aggregate producer throughput.
    pub(crate) limiter: Option<RateLimiter>,
    /// Where discarded frames go, and how their loss is counted and logged.
    pub(crate) dead_letter: DeadLetterState<F>,
    /// What this handle's awaited pushes had to wait for, across both lanes.
    pub(crate) waits: PushWaitCounts,
    /// Payload-size budgets checked before a frame is queued.
    pub(crate) frame_limits: FrameLimits<F>,
}

/// Cloneable handle used by producers to push frames to a connection.
pub struct PushHandle<F>(Arc<PushHandleInner<F>>);

// Written by hand so cloning a handle does not require the frame type itself to
// be `Clone`: clones share the same queues and only bump the inner `Arc`.
impl<F> Clone for PushHandle<F> {
    fn clone(&self) -> Self { Self(Arc::clone(&self.0)) }
}

impl<F: FrameLike> PushHandle<F> {
    /// Wrap shared queue state while retaining cloneable producer ownership.
    pub(crate) const fn from_arc(arc: Arc<PushHandleInner<F>>) -> Self { Self(arc) }

    /// Returns a probe for inspecting internal state during loom verification.
    ///
    /// Exposes dead-letter drop counters so loom tests can assert behaviour
    /// without altering the production API.
    #[cfg(loom)]
    #[must_use]
    pub fn probe(&self) -> PushHandleProbe<F> { PushHandleProbe::new(self.0.clone()) }

    /// Internal helper to push a frame with the requested priority.
    ///
    /// Capacity is reserved before a rate permit is drawn. A permit taken
    /// first would let several producers park holding one each, and those
    /// parked frames would then enter the queue back-to-back as soon as the
    /// consumer drained it, exceeding the configured rate on the wire. Holding
    /// a reservation instead delays only the producers that have nowhere to
    /// put a frame, and every frame still enters the queue at the configured
    /// rate.
    ///
    /// The limiter is never awaited directly: awaiting it registers this task
    /// as a waiter and reserves a token even if the future is never polled
    /// again (for example after a `now_or_never()` probe in tests), which can
    /// starve other pushes. A non-blocking check in a short sleep loop keeps
    /// tokens available to actively polled tasks.
    async fn push_with_priority(&self, frame: F, priority: PushPriority) -> Result<(), PushError> {
        self.check_frame_len(&frame)?;
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };

        // A closed connection ends the producer's work, so report that rather
        // than waiting for the rate limiter to refill first.
        if tx.is_closed() {
            return Err(PushError::Closed);
        }

        // Reserve a slot first: awaiting capacity if the queue is currently
        // full, and surfacing `Closed` if the receiver has been dropped.
        //
        // The non-blocking probe below is what makes "did this push wait?"
        // observable. It reserves a slot if one is free, so the common case
        // still completes without parking. Probing on every send costs one
        // atomic acquisition, which is noise beside the channel operations it
        // guards, and it keeps the counter meaningful for any caller rather
        // than only for instrumentation that opted in.
        //
        // A refusal is *not* by itself a wait. It says one poll of `reserve`
        // would park, but nothing has polled yet, and a caller that frees a
        // slot in between gets a reservation that never suspends. Counting the
        // refusal would make the counter report a suspension that did not
        // happen — which is exactly what a back-pressure observer watching
        // this counter would then mistake for proof the producer had parked.
        // So the wait is recorded on the poll that actually returns `Pending`,
        // once per send, and only then. `reserve_capacity` is what watches for
        // that poll.
        self.0.waits.record_capacity_probe();
        let permit = match tx.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(())) => self.reserve_capacity(tx).await?,
            Err(mpsc::error::TrySendError::Closed(())) => return Err(PushError::Closed),
        };

        if let Some(ref limiter) = self.0.limiter {
            self.wait_for_permit(limiter, tx).await?;
        }
        permit.send(frame);
        debug!("frame pushed: priority={priority:?}");
        Ok(())
    }

    /// Push a high-priority frame subject to rate limiting.
    ///
    /// The call awaits if the rate limiter has no available tokens or
    /// the queue is full.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::Closed`] if the receiving end has been dropped, and
    /// [`PushError::FrameTooLarge`] if the connection's codec cannot encode
    /// the frame.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tokio::runtime::Runtime;
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// let rt = Runtime::new().expect("failed to build runtime");
    /// rt.block_on(async {
    ///     let (mut queues, handle) = PushQueues::<u8>::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .rate(Some(1))
    ///         .build()
    ///         .expect("failed to build PushQueues");
    ///     handle.push_high_priority(42u8).await.expect("push failed");
    ///     let (priority, frame) = queues.recv().await.expect("recv failed");
    ///     assert_eq!(priority, PushPriority::High);
    ///     assert_eq!(frame, 42);
    /// });
    /// ```
    pub async fn push_high_priority(&self, frame: F) -> Result<(), PushError> {
        self.push_with_priority(frame, PushPriority::High).await
    }

    /// Push a low-priority frame subject to rate limiting.
    ///
    /// Awaits if the rate limiter has no available tokens or the queue is full.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::Closed`] if the receiving end has been dropped, and
    /// [`PushError::FrameTooLarge`] if the connection's codec cannot encode
    /// the frame.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tokio::runtime::Runtime;
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// let rt = Runtime::new().expect("failed to build runtime");
    /// rt.block_on(async {
    ///     let (mut queues, handle) = PushQueues::<u8>::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .rate(Some(1))
    ///         .build()
    ///         .expect("failed to build PushQueues");
    ///     handle.push_low_priority(10u8).await.expect("push failed");
    ///     let (priority, frame) = queues.recv().await.expect("recv failed");
    ///     assert_eq!(priority, PushPriority::Low);
    ///     assert_eq!(frame, 10);
    /// });
    /// ```
    pub async fn push_low_priority(&self, frame: F) -> Result<(), PushError> {
        self.push_with_priority(frame, PushPriority::Low).await
    }

    /// Counters describing what this handle's awaited pushes have waited for.
    ///
    /// Every clone of a handle shares one set of counters, so a snapshot taken
    /// here includes pushes made through any clone. The counters only ever
    /// increase; subtract an earlier snapshot with
    /// [`PushWaitCounters::since`] to bracket a region of interest.
    ///
    /// This is the observable that distinguishes an awaited push that waited
    /// for capacity from one that did not, which the non-blocking
    /// `QueueFull` report cannot: that report is produced only by
    /// [`PushHandle::try_push`], and an awaited push against a full queue
    /// returns nothing at all. See [`PushWaitCounters`] for the distinction in
    /// full.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tokio::runtime::Runtime;
    /// use wireframe::push::PushQueues;
    ///
    /// let rt = Runtime::new().expect("failed to build runtime");
    /// rt.block_on(async {
    ///     let (mut queues, handle) = PushQueues::<u8>::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .unlimited()
    ///         .build()
    ///         .expect("failed to build PushQueues");
    ///     let before = handle.wait_counters();
    ///     handle.push_high_priority(7u8).await.expect("push failed");
    ///     let waited = handle.wait_counters().since(before);
    ///     assert_eq!(waited.awaited_capacity_waits, 0);
    ///     assert_eq!(waited.capacity_probes, 1);
    ///     let _ = queues.recv().await;
    /// });
    /// ```
    #[must_use]
    pub fn wait_counters(&self) -> PushWaitCounters { self.0.waits.snapshot() }

    /// Interval between attempts to acquire a rate-limit permit.
    ///
    /// Kept short so tests that use `tokio::time::pause/advance` progress
    /// quickly while remaining negligible relative to the 1s refill window.
    const PERMIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

    /// Reserve a slot on a full channel, counting the wait only if it parks.
    ///
    /// The caller has already probed the channel and found it full, so this is
    /// the cold path. It polls the reservation future itself rather than
    /// awaiting it directly because only the poll result distinguishes the two
    /// outcomes a caller cares about: a slot freed before this future was
    /// first polled yields `Ready` and no wait, while a `Pending` is an actual
    /// suspension. The counter is incremented on the first `Pending` and then
    /// left alone: repetition would count one send once per wake-up.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::Closed`] if the receiver was dropped before a slot
    /// could be reserved.
    async fn reserve_capacity<'a>(
        &self,
        sender: &'a mpsc::Sender<F>,
    ) -> Result<mpsc::Permit<'a, F>, PushError> {
        // Keep the caller's sender borrowed while polling so this slow path
        // does not create a second shared owner just to name the future.
        let mut reserve = std::pin::pin!(sender.reserve());
        let mut is_wait_recorded = false;
        poll_fn(|cx| {
            let polled = reserve.as_mut().poll(cx);
            record_first_pending(&polled, &mut is_wait_recorded, &self.0.waits);
            polled
        })
        .await
        .map_err(|_| PushError::Closed)
    }

    /// Poll permits without registering a parked waiter that could starve pushes.
    ///
    /// A closed channel is reported as such even while the limiter is
    /// exhausted, so a producer waiting for a refill does not outlive the
    /// connection it is pushing to.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::Closed`] once the receiver has been dropped.
    async fn wait_for_permit(
        &self,
        limiter: &RateLimiter,
        sender: &mpsc::Sender<F>,
    ) -> Result<(), PushError> {
        loop {
            if sender.is_closed() {
                return Err(PushError::Closed);
            }
            if limiter.try_acquire(1) {
                return Ok(());
            }
            // The limiter is configured with a 1s refill interval; a short
            // sleep yields to the scheduler and advances virtual time in
            // tests (tokio::time::pause/advance).
            sleep(Self::PERMIT_POLL_INTERVAL).await;
        }
    }

    /// Attempt to push a frame with the given priority and policy.
    ///
    /// Unlike [`push_high_priority`](Self::push_high_priority) and
    /// [`push_low_priority`](Self::push_low_priority), this method never waits:
    /// it makes one non-blocking attempt to acquire a rate-limit permit and one
    /// non-blocking attempt to send. A frame is not queued when the limiter has
    /// no permit, so a producer cannot exceed the configured rate by using this
    /// path.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::RateLimited`] when the shared rate limiter has no
    /// permit available. The frame is then discarded without being queued or
    /// dead-lettered, and the caller may retry after the next refill window.
    /// Returns [`PushError::QueueFull`] if the queue is full and the policy is
    /// [`PushPolicy::ReturnErrorIfFull`]. Returns [`PushError::Closed`] if the
    /// receiving end has been dropped, which is reported even when no permit
    /// is available: a closed connection is permanent, so a producer must not
    /// be told to retry after a refill. Returns
    /// [`PushError::FrameTooLarge`] if the connection's codec cannot encode the
    /// frame. When
    /// [`PushPolicy::DropIfFull`] or [`PushPolicy::WarnAndDropIfFull`] is used,
    /// a configured dead letter queue receives the dropped frame.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tokio::{runtime::Runtime, sync::mpsc};
    /// use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};
    ///
    /// let rt = Runtime::new().expect("failed to build runtime");
    /// rt.block_on(async {
    ///     let (dlq_tx, mut dlq_rx) = mpsc::channel(1);
    ///     let (mut queues, handle) = PushQueues::<u8>::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .rate(None)
    ///         .dlq(Some(dlq_tx))
    ///         .build()
    ///         .expect("failed to build PushQueues");
    ///     handle.push_high_priority(1u8).await.expect("push failed");
    ///
    ///     handle
    ///         .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
    ///         .expect("try_push failed");
    ///
    ///     assert_eq!(dlq_rx.recv().await.expect("recv failed"), 2);
    ///     let _ = queues.recv().await;
    /// });
    /// ```
    pub fn try_push(
        &self,
        frame: F,
        priority: PushPriority,
        policy: PushPolicy,
    ) -> Result<(), PushError>
    where
        F: std::fmt::Debug,
    {
        self.check_frame_len(&frame)?;
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };
        // Closure is checked before the rate limiter: an exhausted permit
        // reports `RateLimited`, and a producer that retried on that would
        // outlive a connection that has already ended.
        if tx.is_closed() {
            return Err(PushError::Closed);
        }
        // The shared limiter is consulted here too, so this path cannot
        // overdraw the budget the awaited pushes draw on. It cannot wait, so a
        // missing permit is a refusal rather than a delay.
        if let Some(ref limiter) = self.0.limiter
            && !limiter.try_acquire(1)
        {
            // The caller may discard the error, so this is the only aggregate
            // signal that a push was refused for rate rather than for capacity.
            crate::metrics::inc_push_rate_limited();
            return Err(PushError::RateLimited);
        }

        match tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(f)) => match policy {
                PushPolicy::ReturnErrorIfFull => Err(PushError::QueueFull),
                PushPolicy::DropIfFull | PushPolicy::WarnAndDropIfFull => {
                    if matches!(policy, PushPolicy::WarnAndDropIfFull) {
                        warn!(
                            "push queue full: priority={priority:?}, policy={policy:?}, dlq={dlq}",
                            dlq = self.0.dead_letter.has_queue()
                        );
                    }
                    self.0.dead_letter.route_or_count(f, priority, policy);
                    Ok(())
                }
            },
            Err(mpsc::error::TrySendError::Closed(_)) => Err(PushError::Closed),
        }
    }

    /// Reject a frame the connection's codec could not encode.
    ///
    /// Checking at enqueue keeps an unencodable push from being reported as
    /// accepted and then ending the connection in the encoder. A handle built
    /// without a limit accepts every frame, because only the application knows
    /// which codec will write them.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::FrameTooLarge`] when the payload exceeds the limit.
    fn check_frame_len(&self, frame: &F) -> Result<(), PushError> {
        self.0.frame_limits.check(frame)
    }

    /// Downgrade to a `Weak` reference for storage in a registry.
    pub(crate) fn downgrade(&self) -> Weak<PushHandleInner<F>> { Arc::downgrade(&self.0) }
}
