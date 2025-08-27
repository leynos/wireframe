//! Queue management used by [`PushHandle`] and [`PushQueues`].
//!
//! Provides the core implementation for prioritised queues delivering frames
//! to a connection. Background tasks can send messages without blocking the
//! request/response cycle. Frames maintain FIFO order within each priority
//! level. An optional rate limiter caps throughput at [`MAX_PUSH_RATE`] pushes
//! per second.

use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use leaky_bucket::RateLimiter;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

/// Messages can be sent through a [`PushHandle`].
///
/// The trait is intentionally empty: any type that is `Send` and `'static`
/// is considered a valid frame for pushing to a connection.
pub trait FrameLike: Send + 'static {}

impl<T> FrameLike for T where T: Send + 'static {}

// Default maximum pushes per second when no custom rate is specified.
// This is an internal implementation detail and may change.
const DEFAULT_PUSH_RATE: usize = 100;
/// Highest supported rate for [`PushQueuesBuilder::rate`].
pub const MAX_PUSH_RATE: usize = 10_000;

// Compile-time guard: DEFAULT_PUSH_RATE must not exceed MAX_PUSH_RATE.
const _: () = assert!(DEFAULT_PUSH_RATE <= MAX_PUSH_RATE);

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

/// Errors returned when creating push queues.
#[derive(Debug)]
pub enum PushConfigError {
    /// The provided rate was zero or exceeded [`MAX_PUSH_RATE`].
    InvalidRate(usize),
}

impl std::fmt::Display for PushConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRate(r) => {
                write!(f, "invalid rate {r}; must be between 1 and {MAX_PUSH_RATE}")
            }
        }
    }
}

impl std::error::Error for PushConfigError {}

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
    high_prio_tx: mpsc::Sender<F>,
    low_prio_tx: mpsc::Sender<F>,
    limiter: Option<RateLimiter>,
    dlq_tx: Option<mpsc::Sender<F>>,
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
    ///     let (mut queues, handle) = PushQueues::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .rate(Some(1))
    ///         .build()
    ///         .unwrap();
    ///     handle.push_high_priority(42u8).await.unwrap();
    ///     let (priority, frame) = queues.recv().await.unwrap();
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
    ///     let (mut queues, handle) = PushQueues::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .rate(Some(1))
    ///         .build()
    ///         .unwrap();
    ///     handle.push_low_priority(10u8).await.unwrap();
    ///     let (priority, frame) = queues.recv().await.unwrap();
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
    ///     let (mut queues, handle) = PushQueues::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .rate(None)
    ///         .dlq(Some(dlq_tx))
    ///         .build()
    ///         .unwrap();
    ///     handle.push_high_priority(1u8).await.unwrap();
    ///
    ///     handle
    ///         .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
    ///         .unwrap();
    ///
    ///     assert_eq!(dlq_rx.recv().await.unwrap(), 2);
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

/// Receiver ends of the push queues stored by the connection actor.
pub struct PushQueues<F> {
    pub(crate) high_priority_rx: mpsc::Receiver<F>,
    pub(crate) low_priority_rx: mpsc::Receiver<F>,
}

/// Builder for [`PushQueues`].
///
/// Allows configuration of queue capacities, rate limiting and an optional
/// dead-letter queue before constructing [`PushQueues`] and its paired
/// [`PushHandle`]. Defaults mirror the previous constructors: both queues have
/// a capacity of one and pushes are limited to [`DEFAULT_PUSH_RATE`] per second
/// unless overridden.
pub struct PushQueuesBuilder<F> {
    high_capacity: usize,
    low_capacity: usize,
    rate: Option<usize>,
    dlq: Option<mpsc::Sender<F>>,
}

impl<F: FrameLike> PushQueuesBuilder<F> {
    fn new() -> Self {
        Self {
            high_capacity: 1,
            low_capacity: 1,
            rate: Some(DEFAULT_PUSH_RATE),
            dlq: None,
        }
    }

    /// Set the capacity of the high-priority queue.
    #[must_use]
    pub fn high_capacity(mut self, capacity: usize) -> Self {
        self.high_capacity = capacity;
        self
    }

    /// Set the capacity of the low-priority queue.
    #[must_use]
    pub fn low_capacity(mut self, capacity: usize) -> Self {
        self.low_capacity = capacity;
        self
    }

    /// Set the global push rate limit in pushes per second.
    #[must_use]
    pub fn rate(mut self, rate: Option<usize>) -> Self {
        self.rate = rate;
        self
    }

    /// Provide a dead-letter queue for discarded frames.
    #[must_use]
    pub fn dlq(mut self, dlq: Option<mpsc::Sender<F>>) -> Self {
        self.dlq = dlq;
        self
    }

    /// Build the configured [`PushQueues`] and associated [`PushHandle`].
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if the rate is zero or above
    /// [`MAX_PUSH_RATE`].
    pub fn build(self) -> Result<(PushQueues<F>, PushHandle<F>), PushConfigError> {
        PushQueues::build_with_rate_dlq(self.high_capacity, self.low_capacity, self.rate, self.dlq)
    }
}

impl<F: FrameLike> PushQueues<F> {
    /// Start building a new set of push queues.
    #[must_use]
    pub fn builder() -> PushQueuesBuilder<F> { PushQueuesBuilder::new() }

    fn build_with_rate_dlq(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
        dlq: Option<mpsc::Sender<F>>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        if let Some(r) = rate.filter(|r| *r == 0 || *r > MAX_PUSH_RATE) {
            // Reject unsupported rates early to avoid building queues that cannot
            // be used. The bounds prevent runaway resource consumption.
            return Err(PushConfigError::InvalidRate(r));
        }
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
            dlq_tx: dlq,
        };
        Ok((
            Self {
                high_priority_rx: high_rx,
                low_priority_rx: low_rx,
            },
            PushHandle(Arc::new(inner)),
        ))
    }

    /// Create a new set of queues with the specified bounds for each priority
    /// and return them along with a [`PushHandle`] for producers.
    ///
    /// # Panics
    ///
    /// Panics if an internal invariant is violated. This should never occur.
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    #[must_use]
    pub fn bounded(high_capacity: usize, low_capacity: usize) -> (Self, PushHandle<F>) {
        Self::builder()
            .high_capacity(high_capacity)
            .low_capacity(low_capacity)
            .build()
            .expect("DEFAULT_PUSH_RATE is always valid")
    }

    /// Create queues with no rate limiting.
    ///
    /// # Panics
    ///
    /// Panics if an internal invariant is violated. This should never occur.
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    #[must_use]
    pub fn bounded_no_rate_limit(
        high_capacity: usize,
        low_capacity: usize,
    ) -> (Self, PushHandle<F>) {
        Self::builder()
            .high_capacity(high_capacity)
            .low_capacity(low_capacity)
            .rate(None)
            .build()
            .expect("bounded_no_rate_limit should not fail")
    }

    /// Create queues with a custom rate limit in pushes per second.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if `rate` is zero or greater
    /// than [`MAX_PUSH_RATE`].
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    pub fn bounded_with_rate(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        Self::builder()
            .high_capacity(high_capacity)
            .low_capacity(low_capacity)
            .rate(rate)
            .build()
    }

    /// Create queues with a custom rate limit and optional dead letter queue.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if `rate` is zero or greater
    /// than [`MAX_PUSH_RATE`].
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    pub fn bounded_with_rate_dlq(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
        dlq: Option<mpsc::Sender<F>>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        Self::build_with_rate_dlq(high_capacity, low_capacity, rate, dlq)
    }

    /// Receive the next frame, preferring high priority frames when available.
    ///
    /// Returns `None` when both queues are closed and empty.
    ///
    /// # Examples
    ///
    /// Note: this method is biased towards high priority traffic and may starve
    /// low priority frames if producers saturate the high queue.
    ///
    /// ```rust,no_run
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
    ///     let (mut queues, handle) = PushQueues::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .build()
    ///         .unwrap();
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
    /// let (mut queues, _handle) = PushQueues::<u8>::builder().build().unwrap();
    /// queues.close();
    /// ```
    pub fn close(&mut self) {
        self.high_priority_rx.close();
        self.low_priority_rx.close();
    }
}
