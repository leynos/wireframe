//! Cloneable handle used by producers to push frames to a connection.

#[cfg(loom)]
mod sync {
    pub use std::sync::{Arc, Weak};

    pub use loom::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
}

#[cfg(not(loom))]
mod sync {
    pub use std::sync::{
        Arc,
        Mutex,
        Weak,
        atomic::{AtomicUsize, Ordering},
    };
}

use std::time::{Duration, Instant};

use leaky_bucket::RateLimiter;
use log::{debug, warn};
use tokio::{sync::mpsc, time::sleep};

use self::sync::{Arc, AtomicUsize, Mutex, Ordering, Weak};
use super::{FrameLike, PushError, PushPolicy, PushPriority};

/// Shared state for [`PushHandle`].
///
/// Holds the high- and low-priority channels alongside an optional rate
/// limiter and dead-letter queue sender used when pushes are discarded.
///
/// - `high_prio_tx` – channel for frames that must be sent before any low-priority traffic.
/// - `low_prio_tx` – channel for best-effort frames.
/// - `limiter` – optional rate-limiter enforcing global push throughput.
/// - `dlq_tx` – optional dead-letter queue for discarded frames.
pub(crate) struct PushHandleInner<F> {
    pub(crate) high_prio_tx: mpsc::Sender<F>,
    pub(crate) low_prio_tx: mpsc::Sender<F>,
    pub(crate) limiter: Option<RateLimiter>,
    pub(crate) dlq_tx: Option<mpsc::Sender<F>>,
    pub(crate) dlq_drops: AtomicUsize,
    pub(crate) dlq_last_log: Mutex<Instant>,
    pub(crate) dlq_log_every_n: usize,
    pub(crate) dlq_log_interval: Duration,
}

/// Cloneable handle used by producers to push frames to a connection.
#[derive(Clone)]
pub struct PushHandle<F>(Arc<PushHandleInner<F>>);

/// Instrumentation helper exposing internal counters when running under loom.
#[cfg(loom)]
pub struct PushHandleProbe<F> {
    inner: Arc<PushHandleInner<F>>,
}

#[cfg(loom)]
impl<F> PushHandleProbe<F> {
    /// Return the number of frames dropped into the DLQ since the last log flush.
    #[must_use]
    pub fn dlq_drop_count(&self) -> usize { self.inner.dlq_drops.load(Ordering::SeqCst) }
}

impl<F: FrameLike> PushHandle<F> {
    pub(crate) fn from_arc(arc: Arc<PushHandleInner<F>>) -> Self { Self(arc) }

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
            loop {
                // Prefer a non-blocking acquisition. If not available, back
                // off briefly before trying again. We intentionally do not
                // poll the limiter's async acquire future to avoid enqueuing
                // this task as a waiter and reserving a token prematurely.
                if limiter.try_acquire(1) {
                    break;
                }
                // The limiter is configured with a 1s refill interval; a
                // short sleep yields to the scheduler and advances virtual
                // time in tests (tokio::time::pause/advance).
                sleep(Duration::from_millis(10)).await;
            }
        }

        // Then send the frame, awaiting capacity if the queue is currently
        // full. If the receiver has been dropped, surface `Closed`.
        tx.clone()
            .send(frame)
            .await
            .map_err(|_| PushError::Closed)?;
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

    /// Send a frame to the configured dead letter queue if available.
    fn route_to_dlq(&self, frame: F)
    where
        F: std::fmt::Debug,
    {
        let log_every_n = self.0.dlq_log_every_n;
        let log_interval = self.0.dlq_log_interval;

        if let Some(dlq) = &self.0.dlq_tx {
            match dlq.try_send(frame) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(f) | mpsc::error::TrySendError::Closed(f)) => {
                    let dropped = self.0.dlq_drops.fetch_add(1, Ordering::Relaxed) + 1;
                    let mut last = self.0.dlq_last_log.lock().expect("lock poisoned");
                    let now = Instant::now();
                    if (log_every_n != 0 && dropped.is_multiple_of(log_every_n))
                        || now.duration_since(*last) > log_interval
                    {
                        warn!(
                            "DLQ dropped frames (full or closed): frame={f:?}, dropped={dropped}, \
                             log_every_n={log_every_n}, log_interval={log_interval:?}"
                        );
                        *last = now;
                        self.0.dlq_drops.store(0, Ordering::Relaxed);
                    }
                }
            }
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
                            dlq = self.0.dlq_tx.is_some()
                        );
                    }
                    self.route_to_dlq(f);
                    Ok(())
                }
            },
            Err(mpsc::error::TrySendError::Closed(_)) => Err(PushError::Closed),
        }
    }

    /// Downgrade to a `Weak` reference for storage in a registry.
    pub(crate) fn downgrade(&self) -> Weak<PushHandleInner<F>> { Arc::downgrade(&self.0) }
}
