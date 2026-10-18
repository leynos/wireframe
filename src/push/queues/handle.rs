//! Cloneable handle used by producers to push frames to a connection.

use std::{
    future::{Future, poll_fn},
    task::Poll,
    time::Duration,
};

use leaky_bucket::RateLimiter;
use log::{debug, warn};
use tokio::{sync::mpsc, time::sleep};

use super::{
    FrameLike,
    PushError,
    PushPolicy,
    PushPriority,
    dlq::DeadLetterState,
    sync::{Arc, Weak},
    waits::{PushWaitCounters, PushWaitCounts},
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
}

/// Cloneable handle used by producers to push frames to a connection.
#[derive(Clone)]
pub struct PushHandle<F>(Arc<PushHandleInner<F>>);

/// Instrumentation helper exposing internal counters when running under loom.
#[cfg(loom)]
pub struct PushHandleProbe<F> {
    /// The handle's shared state, read without taking part in any push.
    inner: Arc<PushHandleInner<F>>,
}

#[cfg(loom)]
impl<F> PushHandleProbe<F> {
    /// Return the number of frames dropped into the DLQ since the last log flush.
    #[must_use]
    pub fn dlq_drop_count(&self) -> usize { self.inner.dead_letter.pending_count() }

    /// Return the total number of drops the throttled diagnostic has reported.
    #[must_use]
    pub fn dlq_reported_count(&self) -> usize { self.inner.dead_letter.reported_count() }
}

impl<F: FrameLike> PushHandle<F> {
    /// Wrap shared queue state while retaining cloneable producer ownership.
    pub(crate) fn from_arc(arc: Arc<PushHandleInner<F>>) -> Self { Self(arc) }

    /// Returns a probe for inspecting internal state during loom verification.
    ///
    /// Exposes dead-letter drop counters so loom tests can assert behaviour
    /// without altering the production API.
    #[cfg(loom)]
    #[must_use]
    pub fn probe(&self) -> PushHandleProbe<F> {
        PushHandleProbe {
            inner: self.0.clone(),
        }
    }

    /// Internal helper to push a frame with the requested priority.
    ///
    /// IMPORTANT: We honour the rate limiter before attempting to reserve
    /// channel capacity, but without awaiting the limiter's future directly.
    /// Awaiting the limiter can register this task as a waiter and reserve
    /// a token even if the future is never polled again (e.g., after a
    /// `now_or_never()` probe in tests). That reservation can starve other
    /// pushes and cause hangs. Instead, we perform a non-blocking check in a
    /// short sleep loop so no capacity or tokens are held while parked.
    async fn push_with_priority(&self, frame: F, priority: PushPriority) -> Result<(), PushError> {
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };

        // First, honour the rate limit without registering this task as a
        // waiter on the limiter. If we await the limiter's future directly,
        // it will queue our waker and reserve the next token for us. In test
        // scenarios that probe with `now_or_never()` and never poll the
        // future again, that reserved token would block unrelated pushes and
        // lead to hangs. By performing a non-blocking check and sleeping until
        // the next refill window, tokens remain available to actively polled
        // tasks.
        if let Some(ref limiter) = self.0.limiter {
            self.wait_for_permit(limiter).await;
        }

        // Then send the frame, awaiting capacity if the queue is currently
        // full. If the receiver has been dropped, surface `Closed`.
        //
        // The non-blocking probe below is what makes "did this push wait?"
        // observable. It reserves a slot if one is free, so the common case
        // still completes without parking. Probing on every send costs one
        // atomic acquisition, which is noise beside the channel operations it
        // guards, and it keeps the counter meaningful for any caller rather
        // than only for instrumentation that opted in.
        //
        // A refusal is *not* by itself a wait. It says one poll of `send`
        // would park, but nothing has polled yet, and a caller that frees a
        // slot in between gets a send that never suspends. Counting the
        // refusal would make the counter report a suspension that did not
        // happen — which is exactly what a back-pressure observer watching
        // this counter would then mistake for proof the producer had parked.
        // So the wait is recorded on the poll that actually returns `Pending`,
        // once per send, and only then. `await_send_capacity` is what watches
        // for that poll.
        self.0.waits.record_capacity_probe();
        match tx.try_reserve() {
            Ok(permit) => permit.send(frame),
            Err(mpsc::error::TrySendError::Full(())) => {
                self.await_send_capacity(tx.clone(), frame).await?;
            }
            Err(mpsc::error::TrySendError::Closed(())) => return Err(PushError::Closed),
        }
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
    /// Returns [`PushError::Closed`] if the receiving end has been dropped.
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
    /// Returns [`PushError::Closed`] if the receiving end has been dropped.
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

    /// Send through a channel that was full, counting the wait only if it parks.
    ///
    /// The caller has already reserved and found the channel full, so this is
    /// the cold path. It polls the send future itself rather than awaiting it
    /// directly because only the poll result distinguishes the two outcomes a
    /// caller cares about: a slot freed before this future was first polled
    /// yields `Ready` and no wait, while a `Pending` is an actual suspension.
    /// The counter is incremented on the first `Pending` and then left alone:
    /// repetition would count one send once per wake-up.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::Closed`] if the receiver was dropped before the
    /// frame could be sent.
    async fn await_send_capacity(
        &self,
        sender: mpsc::Sender<F>,
        frame: F,
    ) -> Result<(), PushError> {
        // `Sender::send` borrows its receiver, so the sender must be a named
        // binding: pinning `tx.clone().send(frame)` would hold a borrow of a
        // temporary that ends with the statement (E0716) before the future is
        // ever polled.
        let mut send = std::pin::pin!(sender.send(frame));
        let mut is_wait_recorded = false;
        poll_fn(|cx| {
            let polled = send.as_mut().poll(cx);
            Self::record_first_pending(&polled, &mut is_wait_recorded, &self.0.waits);
            polled
        })
        .await
        .map_err(|_| PushError::Closed)
    }

    /// Count one capacity wait the first time a send poll reports `Pending`.
    ///
    /// Split out of the `poll_fn` closure so the closure stays a poll and a
    /// call, keeping it under the repository's nesting ceiling.
    fn record_first_pending(
        polled: &Poll<Result<(), mpsc::error::SendError<F>>>,
        is_wait_recorded: &mut bool,
        waits: &PushWaitCounts,
    ) {
        if polled.is_pending() && !*is_wait_recorded {
            *is_wait_recorded = true;
            waits.record_capacity_wait();
        }
    }

    /// Poll permits without registering a parked waiter that could starve pushes.
    async fn wait_for_permit(&self, limiter: &RateLimiter) {
        loop {
            if limiter.try_acquire(1) {
                break;
            }
            // The limiter is configured with a 1s refill interval; a short
            // sleep yields to the scheduler and advances virtual time in
            // tests (tokio::time::pause/advance).
            sleep(Self::PERMIT_POLL_INTERVAL).await;
        }
    }

    /// Attempt to push a frame with the given priority and policy.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::QueueFull`] if the queue is full and the policy is
    /// [`PushPolicy::ReturnErrorIfFull`]. Returns [`PushError::Closed`] if the
    /// receiving end has been dropped. When [`PushPolicy::DropIfFull`] or
    /// [`PushPolicy::WarnAndDropIfFull`] is used, a configured dead letter queue
    /// receives the dropped frame.
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
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };

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
                    self.0.dead_letter.route_or_count(f);
                    Ok(())
                }
            },
            Err(mpsc::error::TrySendError::Closed(_)) => Err(PushError::Closed),
        }
    }

    /// Downgrade to a `Weak` reference for storage in a registry.
    pub(crate) fn downgrade(&self) -> Weak<PushHandleInner<F>> { Arc::downgrade(&self.0) }
}
