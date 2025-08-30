//! Cloneable handle used by producers to push frames to a connection.

use std::sync::{Arc, Weak};

use leaky_bucket::RateLimiter;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

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
}

/// Cloneable handle used by producers to push frames to a connection.
#[derive(Clone)]
pub struct PushHandle<F>(Arc<PushHandleInner<F>>);

impl<F: FrameLike> PushHandle<F> {
    pub(crate) fn from_arc(arc: Arc<PushHandleInner<F>>) -> Self { Self(arc) }

    /// Internal helper to push a frame with the requested priority.
    ///
    /// Waits on the rate limiter if configured and sends the frame to the
    /// appropriate channel, mapping send errors to [`PushError`].
    async fn push_with_priority(&self, frame: F, priority: PushPriority) -> Result<(), PushError> {
        if let Some(ref limiter) = self.0.limiter {
            limiter.acquire(1).await;
        }
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };
        tx.send(frame).await.map_err(|_| PushError::Closed)?;
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
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
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
    /// }
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
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
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
    /// }
    /// ```
    pub async fn push_low_priority(&self, frame: F) -> Result<(), PushError> {
        self.push_with_priority(frame, PushPriority::Low).await
    }

    /// Send a frame to the configured dead letter queue if available.
    fn route_to_dlq(&self, frame: F)
    where
        F: std::fmt::Debug,
    {
        if let Some(dlq) = &self.0.dlq_tx {
            match dlq.try_send(frame) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(f)) => {
                    error!(?f, "push queue and DLQ full; frame lost");
                }
                Err(mpsc::error::TrySendError::Closed(f)) => {
                    error!(?f, "DLQ closed; frame lost");
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
    /// use tokio::sync::mpsc;
    /// use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
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
    /// }
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
