//! Cloneable handle used by producers to push frames to a connection.

use std::{
    sync::{
        Arc,
        Mutex,
        Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use leaky_bucket::RateLimiter;
use tokio::sync::mpsc;
use tracing::{debug, warn};

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

impl<F: FrameLike> PushHandle<F> {
    pub(crate) fn from_arc(arc: Arc<PushHandleInner<F>>) -> Self { Self(arc) }

    /// Internal helper to push a frame with the requested priority.
    ///
    /// Reserves queue capacity before waiting on the rate limiter and sending
    /// the frame to the appropriate channel. This avoids delaying when the
    /// queue has been closed.
    async fn push_with_priority(&self, frame: F, priority: PushPriority) -> Result<(), PushError> {
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };

        let permit = tx
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| PushError::Closed)?;

        if let Some(ref limiter) = self.0.limiter {
            limiter.acquire(1).await;
        }

        let returned_tx = permit.send(frame);
        // Receiver may have closed after reserving capacity; treat as a send
        // failure to avoid silently dropping the frame.
        if returned_tx.is_closed() {
            return Err(PushError::Closed);
        }
        debug!(?priority, "frame pushed");
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
                    if dropped.is_multiple_of(log_every_n)
                        || now.duration_since(*last) > log_interval
                    {
                        warn!(
                            ?f,
                            dropped,
                            log_every_n,
                            log_interval = ?log_interval,
                            "DLQ dropped frames (full or closed)"
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
                            ?priority,
                            ?policy,
                            dlq = self.0.dlq_tx.is_some(),
                            "push queue full"
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
