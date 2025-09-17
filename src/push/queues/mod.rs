//! Queue management used by [`PushHandle`] and [`PushQueues`].
//!
//! Provides the core implementation for prioritized queues delivering frames
//! to a connection. Background tasks can send messages without blocking the
//! request/response cycle. Frames maintain FIFO order within each priority
//! level. An optional rate limiter caps throughput at [`MAX_PUSH_RATE`] pushes
//! per second.

#[cfg(not(loom))]
use std::sync::{Mutex, atomic::AtomicUsize};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use leaky_bucket::RateLimiter;
#[cfg(loom)]
use loom::sync::Mutex;
#[cfg(loom)]
use loom::sync::atomic::AtomicUsize;
use static_assertions::const_assert;
use tokio::sync::mpsc;

mod builder;
mod errors;
mod handle;

pub use builder::PushQueuesBuilder;
pub use errors::{PushConfigError, PushError};
pub use handle::PushHandle;
pub(crate) use handle::PushHandleInner;

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
const_assert!(DEFAULT_PUSH_RATE <= MAX_PUSH_RATE);

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

/// Receiver ends of the push queues stored by the connection actor.
pub struct PushQueues<F> {
    pub(crate) high_priority_rx: mpsc::Receiver<F>,
    pub(crate) low_priority_rx: mpsc::Receiver<F>,
}

/// Configuration for building [`PushQueues`].
///
/// Bundles capacities, optional rate limiting and dead-letter queue settings,
/// plus logging cadence for dropped frames.
#[derive(Debug, Clone)]
pub struct PushQueueConfig<F> {
    pub high_capacity: usize,
    pub low_capacity: usize,
    pub rate: Option<usize>,
    pub dlq: Option<mpsc::Sender<F>>,
    pub dlq_log_every_n: usize,
    pub dlq_log_interval: Duration,
}

impl<F> PushQueueConfig<F> {
    /// Create a new configuration with default dead-letter queue logging
    /// cadence.
    #[must_use]
    pub fn new(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
        dlq: Option<mpsc::Sender<F>>,
    ) -> Self {
        Self {
            high_capacity,
            low_capacity,
            rate,
            dlq,
            // Log every DLQ drop by default so tests and development
            // environments observe issues immediately. Production users can
            // override via the builder to reduce verbosity.
            dlq_log_every_n: 1,
            dlq_log_interval: Duration::from_secs(10),
        }
    }
}

impl<F: FrameLike> PushQueues<F> {
    /// Start building a new set of push queues.
    #[must_use]
    pub fn builder() -> PushQueuesBuilder<F> { PushQueuesBuilder::default() }

    /// Validates whether the provided rate is invalid (zero or exceeds the maximum).
    fn is_invalid_rate(rate: Option<usize>) -> bool {
        matches!(rate, Some(r) if r == 0 || r > MAX_PUSH_RATE)
    }

    pub(super) fn build_with_config(
        config: PushQueueConfig<F>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        let PushQueueConfig {
            high_capacity,
            low_capacity,
            rate,
            dlq,
            dlq_log_every_n,
            dlq_log_interval,
        } = config;
        if Self::is_invalid_rate(rate) {
            // Reject unsupported rates early to avoid building queues that cannot
            // be used. The bounds prevent runaway resource consumption.
            let r = rate.unwrap();
            return Err(PushConfigError::InvalidRate(r));
        }
        if high_capacity == 0 || low_capacity == 0 {
            return Err(PushConfigError::InvalidCapacity {
                high: high_capacity,
                low: low_capacity,
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
            high_prio_tx: high_tx,
            low_prio_tx: low_tx,
            limiter,
            dlq_tx: dlq,
            dlq_drops: AtomicUsize::new(0),
            dlq_last_log: Mutex::new(Instant::now()),
            dlq_log_every_n,
            dlq_log_interval,
        };
        Ok((
            Self {
                high_priority_rx: high_rx,
                low_priority_rx: low_rx,
            },
            PushHandle::from_arc(Arc::new(inner)),
        ))
    }

    /// Build queues via the public builder API.
    ///
    /// Deprecated constructors share the same configuration pattern; this
    /// helper encapsulates it to keep their implementations concise.
    fn build_via_builder(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
        dlq: Option<mpsc::Sender<F>>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        Self::builder()
            .high_capacity(high_capacity)
            .low_capacity(low_capacity)
            .rate(rate)
            .dlq(dlq)
            .build()
    }

    /// Create a new set of queues with the specified bounds for each priority
    /// and return them along with a [`PushHandle`] for producers.
    ///
    /// # Panics
    ///
    /// Panics if either queue capacity is zero. Prefer `PushQueues::builder()`
    /// to receive a [`Result`] instead.
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    #[must_use]
    pub fn bounded(high_capacity: usize, low_capacity: usize) -> (Self, PushHandle<F>) {
        Self::build_via_builder(high_capacity, low_capacity, Some(DEFAULT_PUSH_RATE), None)
            .expect("invalid capacities or rate in deprecated bounded()")
    }

    /// Create queues with no rate limiting.
    ///
    /// # Panics
    ///
    /// Panics if either queue capacity is zero. Prefer `PushQueues::builder()`
    /// to receive a [`Result`] instead.
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    #[must_use]
    pub fn bounded_no_rate_limit(
        high_capacity: usize,
        low_capacity: usize,
    ) -> (Self, PushHandle<F>) {
        Self::build_via_builder(high_capacity, low_capacity, None, None)
            .expect("invalid capacities in deprecated bounded_no_rate_limit()")
    }

    /// Create queues with a custom rate limit in pushes per second.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if `rate` is zero or greater
    /// than [`MAX_PUSH_RATE`] and [`PushConfigError::InvalidCapacity`] if either
    /// queue capacity is zero.
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    pub fn bounded_with_rate(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        Self::build_via_builder(high_capacity, low_capacity, rate, None)
    }

    /// Create queues with a custom rate limit and optional dead letter queue.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if `rate` is zero or greater
    /// than [`MAX_PUSH_RATE`] and [`PushConfigError::InvalidCapacity`] if either
    /// queue capacity is zero.
    #[deprecated(since = "0.1.0", note = "Use `PushQueues::builder` instead")]
    pub fn bounded_with_rate_dlq(
        high_capacity: usize,
        low_capacity: usize,
        rate: Option<usize>,
        dlq: Option<mpsc::Sender<F>>,
    ) -> Result<(Self, PushHandle<F>), PushConfigError> {
        Self::build_with_config(PushQueueConfig::new(high_capacity, low_capacity, rate, dlq))
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
    /// async fn example() {
    ///     let (mut queues, handle) = PushQueues::<u8>::builder()
    ///         .high_capacity(1)
    ///         .low_capacity(1)
    ///         .build()
    ///         .expect("failed to build PushQueues");
    ///     handle.push_high_priority(2u8).await.expect("push failed");
    ///     let (priority, frame) = queues.recv().await.expect("recv failed");
    ///     assert_eq!(priority, PushPriority::High);
    ///     assert_eq!(frame, 2);
    /// }
    /// ```
    pub async fn recv(&mut self) -> Option<(PushPriority, F)> {
        let mut high_closed = false;
        let mut low_closed = false;
        loop {
            tokio::select! {
                biased;
                res = self.high_priority_rx.recv(), if !high_closed => match res {
                    Some(f) => return Some((PushPriority::High, f)),
                    None => high_closed = true,
                },
                res = self.low_priority_rx.recv(), if !low_closed => match res {
                    Some(f) => return Some((PushPriority::Low, f)),
                    None => low_closed = true,
                },
                else => return None,
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
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// queues.close();
    /// ```
    pub fn close(&mut self) {
        self.high_priority_rx.close();
        self.low_priority_rx.close();
    }
}
