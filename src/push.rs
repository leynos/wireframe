//! Prioritized queues used for asynchronously pushing frames to a connection.
//!
//! `PushQueues` maintain separate high- and low-priority channels so
//! background tasks can send messages without blocking the request/response
//! cycle. Producers interact with these queues through a cloneable
//! [`PushHandle`]. Queued frames are delivered in FIFO order within each
//! priority level.

use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use leaky_bucket::RateLimiter;
use tokio::sync::mpsc;

/// Messages can be sent through a [`PushHandle`].
///
/// The trait is intentionally empty: any type that is `Send` and `'static`
/// is considered a valid frame for pushing to a connection.
pub trait FrameLike: Send + 'static {}

impl<T> FrameLike for T where T: Send + 'static {}

/// Default maximum pushes allowed per second when no custom rate is specified.
const DEFAULT_PUSH_RATE: usize = 100;

/// Priority level for outbound messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushPriority {
    High,
    Low,
}

/// Behaviour when a push queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushPolicy {
    /// Return an error to the caller if the queue is full.
    ReturnErrorIfFull,
    /// Silently drop the frame.
    DropIfFull,
    /// Drop the frame but emit a log warning.
    WarnAndDropIfFull,
}

/// Errors that can occur when pushing a frame.
#[derive(Debug)]
pub enum PushError {
    /// The queue was at capacity and the policy was `ReturnErrorIfFull`.
    QueueFull,
    /// The receiving end of the queue has been dropped.
    Closed,
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => f.write_str("push queue full"),
            Self::Closed => f.write_str("push queue closed"),
        }
    }
}

impl std::error::Error for PushError {}

pub(crate) struct PushHandleInner<F> {
    high_prio_tx: mpsc::Sender<F>,
    low_prio_tx: mpsc::Sender<F>,
    limiter: Option<RateLimiter>,
}

/// Cloneable handle used by producers to push frames to a connection.
#[derive(Clone)]
pub struct PushHandle<F>(Arc<PushHandleInner<F>>);

impl<F: FrameLike> PushHandle<F> {
    pub(crate) fn from_arc(arc: Arc<PushHandleInner<F>>) -> Self { Self(arc) }
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
    ///     let (mut queues, handle) = PushQueues::bounded_with_rate(1, 1, Some(1));
    ///     handle.push_high_priority(42u8).await.unwrap();
    ///     let (priority, frame) = queues.recv().await.unwrap();
    ///     assert_eq!(priority, PushPriority::High);
    ///     assert_eq!(frame, 42);
    /// }
    /// ```
    pub async fn push_high_priority(&self, frame: F) -> Result<(), PushError> {
        if let Some(ref limiter) = self.0.limiter {
            limiter.acquire(1).await;
        }
        self.0
            .high_prio_tx
            .send(frame)
            .await
            .map_err(|_| PushError::Closed)
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
    ///     let (mut queues, handle) = PushQueues::bounded_with_rate(1, 1, Some(1));
    ///     handle.push_low_priority(10u8).await.unwrap();
    ///     let (priority, frame) = queues.recv().await.unwrap();
    ///     assert_eq!(priority, PushPriority::Low);
    ///     assert_eq!(frame, 10);
    /// }
    /// ```
    pub async fn push_low_priority(&self, frame: F) -> Result<(), PushError> {
        if let Some(ref limiter) = self.0.limiter {
            limiter.acquire(1).await;
        }
        self.0
            .low_prio_tx
            .send(frame)
            .await
            .map_err(|_| PushError::Closed)
    }

    /// Attempt to push a frame with the given priority and policy.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::QueueFull`] if the queue is full and the policy is
    /// [`PushPolicy::ReturnErrorIfFull`]. Returns [`PushError::Closed`] if the
    /// receiving end has been dropped.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
    ///     let (mut queues, handle) = PushQueues::bounded(1, 1);
    ///     handle.push_high_priority(1u8).await.unwrap();
    ///
    ///     let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    ///     assert!(matches!(result, Err(PushError::QueueFull)));
    ///     let _ = queues.recv().await;
    /// }
    /// ```
    pub fn try_push(
        &self,
        frame: F,
        priority: PushPriority,
        policy: PushPolicy,
    ) -> Result<(), PushError> {
        let tx = match priority {
            PushPriority::High => &self.0.high_prio_tx,
            PushPriority::Low => &self.0.low_prio_tx,
        };

        match tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_f)) => match policy {
                PushPolicy::ReturnErrorIfFull => Err(PushError::QueueFull),
                PushPolicy::DropIfFull => Ok(()),
                PushPolicy::WarnAndDropIfFull => {
                    log::warn!("push queue full; dropping {priority:?} priority frame");
                    Ok(())
                }
            },
            Err(mpsc::error::TrySendError::Closed(_)) => Err(PushError::Closed),
        }
    }

    /// Downgrade to a `Weak` reference for storage in a registry.
    pub(crate) fn downgrade(&self) -> Weak<PushHandleInner<F>> { Arc::downgrade(&self.0) }
}

/// Receiver ends of the push queues stored by the connection actor.
pub struct PushQueues<F> {
    pub(crate) high_priority_rx: mpsc::Receiver<F>,
    pub(crate) low_priority_rx: mpsc::Receiver<F>,
}

impl<F: FrameLike> PushQueues<F> {
    /// Create a new set of queues with the specified bounds for each priority
    /// and return them along with a [`PushHandle`] for producers.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
    ///     let (mut queues, handle) = PushQueues::<u8>::bounded(1, 1);
    ///     handle.push_high_priority(7u8).await.unwrap();
    ///     let (priority, frame) = queues.recv().await.unwrap();
    ///     assert_eq!(priority, PushPriority::High);
    ///     assert_eq!(frame, 7);
    /// }
    /// ```
    #[must_use]
    pub fn bounded(high_capacity: usize, low_capacity: usize) -> (Self, PushHandle<F>) {
        Self::bounded_with_rate(high_capacity, low_capacity, Some(DEFAULT_PUSH_RATE))
    }

    /// Create queues without any rate limiting.
    #[must_use]
    pub fn bounded_unlimited(high_capacity: usize, low_capacity: usize) -> (Self, PushHandle<F>) {
        Self::bounded_with_rate(high_capacity, low_capacity, None)
    }

    /// Create queues with a custom rate limit in pushes per second.
    ///
    /// The limiter enforces fairness by allowing at most `rate` pushes
    /// per second across all producers for the returned [`PushHandle`].
    /// Pass `None` to disable rate limiting entirely.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::push::PushQueues;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (mut queues, handle) = PushQueues::<u8>::bounded_with_rate(1, 1, Some(10));
    ///     handle.push_low_priority(1u8).await.unwrap();
    ///     let (_prio, frame) = queues.recv().await.unwrap();
    ///     assert_eq!(frame, 1);
    /// }
    /// ```
    #[must_use]
    pub fn bounded_with_rate(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
    ) -> (Self, PushHandle<F>) {
        let (high_tx, high_rx) = mpsc::channel(high_capacity);
        let (low_tx, low_rx) = mpsc::channel(low_capacity);
        let limiter = rate.map(|r| {
            RateLimiter::builder()
                .initial(r)
                .refill(r)
                .interval(Duration::from_secs(1))
                .max(r)
                .build()
        });
        let inner = PushHandleInner {
            high_prio_tx: high_tx,
            low_prio_tx: low_tx,
            limiter,
        };
        (
            Self {
                high_priority_rx: high_rx,
                low_priority_rx: low_rx,
            },
            PushHandle(Arc::new(inner)),
        )
    }

    /// Receive the next frame, preferring high priority frames when available.
    ///
    /// Returns `None` when both queues are closed and empty.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
    ///     let (mut queues, handle) = PushQueues::bounded(1, 1);
    ///     handle.push_high_priority(2u8).await.unwrap();
    ///     let (priority, frame) = queues.recv().await.unwrap();
    ///     assert_eq!(priority, PushPriority::High);
    ///     assert_eq!(frame, 2);
    /// }
    /// ```
    pub async fn recv(&mut self) -> Option<(PushPriority, F)> {
        tokio::select! {
            biased;
            res = self.high_priority_rx.recv() => res.map(|f| (PushPriority::High, f)),
            res = self.low_priority_rx.recv() => res.map(|f| (PushPriority::Low, f)),
        }
    }

    /// Close both receivers to prevent further pushes from being accepted.
    ///
    /// This is primarily used in tests to release resources when no actor is
    /// draining the queues.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::push::PushQueues;
    ///
    /// let (mut queues, _handle) = PushQueues::<u8>::bounded(1, 1);
    /// queues.close();
    /// ```
    pub fn close(&mut self) {
        self.high_priority_rx.close();
        self.low_priority_rx.close();
    }
}
