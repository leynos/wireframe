//! Prioritized queues used for asynchronously pushing frames to a connection.
//!
//! `PushQueues` maintain separate high- and low-priority channels so
//! background tasks can send messages without blocking the request/response
//! cycle. Producers interact with these queues through a cloneable
//! [`PushHandle`]. Queued frames are delivered in FIFO order within each
//! priority level. An optional rate limiter caps throughput at
//! [`MAX_PUSH_RATE`] pushes per second.

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
/// Highest allowed queue capacity for [`PushQueuesBuilder`].
pub const MAX_QUEUE_CAPACITY: usize = 10_000;
/// Maximum burst of high-priority frames before yielding to low priority.
const HIGH_PRIORITY_BURST_LIMIT: usize = 8;

// Compile-time guard: DEFAULT_PUSH_RATE must not exceed MAX_PUSH_RATE.
const _: usize = MAX_PUSH_RATE - DEFAULT_PUSH_RATE;

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
    /// A queue capacity was zero or exceeded [`MAX_QUEUE_CAPACITY`].
    InvalidCapacity {
        queue: PushPriority,
        capacity: usize,
    },
}

impl std::fmt::Display for PushConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRate(r) => {
                write!(f, "invalid rate {r}; must be between 1 and {MAX_PUSH_RATE}")
            }
            Self::InvalidCapacity { queue, capacity } => {
                write!(
                    f,
                    "invalid {queue:?} capacity {capacity}; must be between 1 and \
                     {MAX_QUEUE_CAPACITY}"
                )
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
/// - `high_priority_tx` – channel for frames that must be sent before any low-priority traffic.
/// - `low_priority_tx` – channel for best-effort frames.
/// - `limiter` – optional rate-limiter enforcing global push throughput.
/// - `dlq_tx` – optional dead-letter queue for discarded frames.
pub(crate) struct PushHandleInner<F> {
    high_priority_tx: mpsc::Sender<F>,
    low_priority_tx: mpsc::Sender<F>,
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
            PushPriority::High => &self.0.high_priority_tx,
            PushPriority::Low => &self.0.low_priority_tx,
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
    ///         .capacity(1, 1)
    ///         .rate(Some(1))
    ///         .build()
    ///         .expect("builder() should accept rate within bounds");
    ///     handle
    ///         .push_high_priority(42u8)
    ///         .await
    ///         .expect("push should succeed");
    ///     let (priority, frame) = queues.recv().await.expect("queues should yield frame");
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
    ///         .capacity(1, 1)
    ///         .rate(Some(1))
    ///         .build()
    ///         .expect("builder() should accept rate within bounds");
    ///     handle
    ///         .push_low_priority(10u8)
    ///         .await
    ///         .expect("push should succeed");
    ///     let (priority, frame) = queues.recv().await.expect("queues should yield frame");
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
    ///         .capacity(1, 1)
    ///         .no_rate_limit()
    ///         .with_dlq(dlq_tx)
    ///         .build()
    ///         .expect("builder() should accept DLQ settings");
    ///     handle
    ///         .push_high_priority(1u8)
    ///         .await
    ///         .expect("push should succeed");
    ///
    ///     handle
    ///         .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
    ///         .expect("try_push should queue frame or send to DLQ");
    ///
    ///     assert_eq!(dlq_rx.recv().await.expect("DLQ should receive frame"), 2);
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
            PushPriority::High => &self.0.high_priority_tx,
            PushPriority::Low => &self.0.low_priority_tx,
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

/// Builder for [`PushQueues`].
#[derive(Debug)]
pub struct PushQueuesBuilder<F> {
    high_capacity: usize,
    low_capacity: usize,
    rate: Option<usize>,
    dlq: Option<mpsc::Sender<F>>,
}

impl<F> Default for PushQueuesBuilder<F> {
    fn default() -> Self {
        Self {
            high_capacity: 1,
            low_capacity: 1,
            rate: Some(DEFAULT_PUSH_RATE),
            dlq: None,
        }
    }
}

impl<F: FrameLike> PushQueuesBuilder<F> {
    /// Set the high-priority queue capacity.
    #[must_use]
    pub fn high_capacity(mut self, capacity: usize) -> Self {
        self.high_capacity = capacity;
        self
    }

    /// Set the low-priority queue capacity.
    #[must_use]
    pub fn low_capacity(mut self, capacity: usize) -> Self {
        self.low_capacity = capacity;
        self
    }

    /// Set both high- and low-priority queue capacities.
    #[must_use]
    pub fn capacity(mut self, high: usize, low: usize) -> Self {
        self.high_capacity = high;
        self.low_capacity = low;
        self
    }

    /// Configure a global push rate limit.
    #[must_use]
    pub fn rate(mut self, rate: Option<usize>) -> Self {
        self.rate = rate;
        self
    }

    /// Disable rate limiting entirely.
    #[must_use]
    pub fn no_rate_limit(mut self) -> Self {
        self.rate = None;
        self
    }

    /// Provide a dead-letter queue sender for discarded frames.
    #[must_use]
    pub fn dlq(mut self, dlq: Option<mpsc::Sender<F>>) -> Self {
        self.dlq = dlq;
        self
    }

    /// Configure a dead-letter queue sender for discarded frames.
    #[must_use]
    pub fn with_dlq(mut self, dlq: mpsc::Sender<F>) -> Self {
        self.dlq = Some(dlq);
        self
    }

    /// Build queues with the configured settings.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if `rate` is zero or exceeds
    /// [`MAX_PUSH_RATE`]. Returns [`PushConfigError::InvalidCapacity`] if either
    /// queue capacity is zero or exceeds [`MAX_QUEUE_CAPACITY`].
    pub fn build(self) -> Result<(PushQueues<F>, PushHandle<F>), PushConfigError> {
        PushQueues::build_with_options(self.high_capacity, self.low_capacity, self.rate, self.dlq)
    }
}

/// Receiver ends of the push queues stored by the connection actor.
pub struct PushQueues<F> {
    pub(crate) high_priority_rx: mpsc::Receiver<F>,
    pub(crate) low_priority_rx: mpsc::Receiver<F>,
    high_priority_streak: usize,
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
    ///     let (mut queues, handle) = PushQueues::<u8>::builder()
    ///         .capacity(1, 1)
    ///         .build()
    ///         .expect("builder() should accept default settings");
    ///     handle
    ///         .push_high_priority(7u8)
    ///         .await
    ///         .expect("push should succeed");
    ///     let (priority, frame) = queues.recv().await.expect("queues should yield frame");
    ///     assert_eq!(priority, PushPriority::High);
    ///     assert_eq!(frame, 7);
    /// }
    /// ```
    /// Begin building a new set of queues.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::push::PushQueues;
    ///
    /// let (_queues, _handle) = PushQueues::<u8>::builder()
    ///     .build()
    ///     .expect("builder() should accept default settings");
    /// ```
    #[must_use]
    pub fn builder() -> PushQueuesBuilder<F> { PushQueuesBuilder::default() }

    fn build_with_options(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
        dlq: Option<mpsc::Sender<F>>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        if let Some(r) = rate.filter(|r| *r == 0 || *r > MAX_PUSH_RATE) {
            return Err(PushConfigError::InvalidRate(r));
        }
        if high_capacity == 0 || high_capacity > MAX_QUEUE_CAPACITY {
            return Err(PushConfigError::InvalidCapacity {
                queue: PushPriority::High,
                capacity: high_capacity,
            });
        }
        if low_capacity == 0 || low_capacity > MAX_QUEUE_CAPACITY {
            return Err(PushConfigError::InvalidCapacity {
                queue: PushPriority::Low,
                capacity: low_capacity,
            });
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
            high_priority_tx: high_tx,
            low_priority_tx: low_tx,
            limiter,
            dlq_tx: dlq,
        };
        Ok((
            Self {
                high_priority_rx: high_rx,
                low_priority_rx: low_rx,
                high_priority_streak: 0,
            },
            PushHandle(Arc::new(inner)),
        ))
    }

    /// Receive the next frame, preferring high priority frames when available.
    ///
    /// High-priority bursts are capped by [`HIGH_PRIORITY_BURST_LIMIT`] to avoid
    /// starving low-priority traffic. Internally this uses a biased
    /// [`tokio::select!`] favouring the high queue, so sustained high-priority
    /// traffic may still delay low-priority frames until the burst limit forces a
    /// yield. Returns `None` when both queues are closed and empty.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::push::{PushPriority, PushQueues};
    ///
    /// #[tokio::test]
    /// async fn example() {
    ///     let (mut queues, handle) = PushQueues::builder()
    ///         .capacity(1, 1)
    ///         .build()
    ///         .expect("builder() should accept default settings");
    ///     handle
    ///         .push_high_priority(2u8)
    ///         .await
    ///         .expect("push should succeed");
    ///     let (priority, frame) = queues.recv().await.expect("queues should yield frame");
    ///     assert_eq!(priority, PushPriority::High);
    ///     assert_eq!(frame, 2);
    /// }
    /// ```
    pub async fn recv(&mut self) -> Option<(PushPriority, F)> {
        if self.high_priority_streak >= HIGH_PRIORITY_BURST_LIMIT
            && let Ok(frame) = self.low_priority_rx.try_recv()
        {
            self.high_priority_streak = 0;
            return Some((PushPriority::Low, frame));
        }
        tokio::select! {
            biased;
            res = self.high_priority_rx.recv() => match res {
                Some(frame) => {
                    self.high_priority_streak += 1;
                    Some((PushPriority::High, frame))
                }
                None => self.low_priority_rx.recv().await.map(|f| (PushPriority::Low, f)),
            },
            res = self.low_priority_rx.recv() => {
                self.high_priority_streak = 0;
                res.map(|f| (PushPriority::Low, f))
            }
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
    /// let (mut queues, _handle) = PushQueues::<u8>::builder()
    ///     .capacity(1, 1)
    ///     .build()
    ///     .expect("builder() should accept default settings");
    /// queues.close();
    /// ```
    pub fn close(&mut self) {
        self.high_priority_rx.close();
        self.low_priority_rx.close();
    }
}
