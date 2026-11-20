//! Cloneable handle used by producers to push frames to a connection.

use leaky_bucket::RateLimiter;
use log::warn;
use tokio::sync::mpsc;

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
    waits::{PushWaitCounters, PushWaitCounts},
};

mod lane;

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

    /// Attempt to push a frame with the given priority and policy.
    ///
    /// Unlike [`push_high_priority`](Self::push_high_priority) and
    /// [`push_low_priority`](Self::push_low_priority), this method never waits:
    /// it makes one non-blocking attempt to reserve queue capacity and, only
    /// then, one non-blocking attempt to acquire a rate-limit permit. Capacity
    /// is reserved before a permit is drawn, matching the awaited helpers, so a
    /// push refused for a full queue spends no rate budget and a rate refusal
    /// releases its reservation without changing the queue. A frame is not
    /// queued when the limiter has no permit, so a producer cannot exceed the
    /// configured rate by using this path.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::RateLimited`] when the queue has room but the
    /// shared rate limiter has no permit available. The reservation is then
    /// released, leaving the queue unchanged, and the frame is discarded
    /// without being queued or dead-lettered; the caller may retry after the
    /// next refill window. Returns [`PushError::QueueFull`] if the queue is
    /// full and the policy is [`PushPolicy::ReturnErrorIfFull`]. Returns
    /// [`PushError::Closed`] if the receiving end has been dropped, which is
    /// reported even when no permit is available: a closed connection is
    /// permanent, so a producer must not be told to retry after a refill.
    /// Returns [`PushError::FrameTooLarge`] if the connection's codec cannot
    /// encode the frame. When [`PushPolicy::DropIfFull`] or
    /// [`PushPolicy::WarnAndDropIfFull`] is used, a configured dead letter
    /// queue receives the dropped frame.
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
        // Capacity is reserved before a permit is drawn, the same order the
        // awaited helpers use. A permit taken for a frame that is then refused
        // would spend budget with nothing queued, and a following push on the
        // other lane could be refused `RateLimited` while that budget was gone.
        match tx.try_reserve() {
            Ok(permit) => {
                // The shared limiter is consulted here too, so this path cannot
                // overdraw the budget the awaited pushes draw on. It cannot
                // wait, so a missing permit is a refusal rather than a delay.
                if let Some(ref limiter) = self.0.limiter
                    && !limiter.try_acquire(1)
                {
                    // The caller may discard the error, so this is the only
                    // aggregate signal that a push was refused for rate rather
                    // than for capacity.
                    crate::metrics::inc_push_rate_limited();
                    // Dropping the reservation releases the slot and leaves the
                    // queue exactly as it was.
                    drop(permit);
                    return Err(PushError::RateLimited);
                }
                permit.send(frame);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(())) => {
                self.apply_full_policy(frame, priority, policy)
            }
            Err(mpsc::error::TrySendError::Closed(())) => Err(PushError::Closed),
        }
    }

    /// Apply the configured policy to a frame refused by a full queue.
    ///
    /// The frame is still owned here, so the drop policies can route it to the
    /// dead-letter queue rather than losing it without a trace.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::QueueFull`] when the policy is
    /// [`PushPolicy::ReturnErrorIfFull`].
    fn apply_full_policy(
        &self,
        frame: F,
        priority: PushPriority,
        policy: PushPolicy,
    ) -> Result<(), PushError> {
        match policy {
            PushPolicy::ReturnErrorIfFull => Err(PushError::QueueFull),
            PushPolicy::DropIfFull | PushPolicy::WarnAndDropIfFull => {
                if matches!(policy, PushPolicy::WarnAndDropIfFull) {
                    warn!(
                        "push queue full: priority={priority:?}, policy={policy:?}, dlq={dlq}",
                        dlq = self.0.dead_letter.has_queue()
                    );
                }
                self.0.dead_letter.route_or_count(frame, priority, policy);
                Ok(())
            }
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
