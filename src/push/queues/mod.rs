//! Queue management used by [`PushHandle`] and [`PushQueues`].
//!
//! Provides the core implementation for prioritised queues delivering frames
//! to a connection. Background tasks can send messages without blocking the
//! request/response cycle. Frames maintain FIFO order within each priority
//! level. An optional rate limiter caps throughput at [`MAX_PUSH_RATE`] pushes
//! per second.

use std::{sync::Arc, time::Duration};

use leaky_bucket::RateLimiter;
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

/// Receiver ends of the push queues stored by the connection actor.
pub struct PushQueues<F> {
    pub(crate) high_priority_rx: mpsc::Receiver<F>,
    pub(crate) low_priority_rx: mpsc::Receiver<F>,
}

impl<F: FrameLike> PushQueues<F> {
    /// Start building a new set of push queues.
    #[must_use]
    pub fn builder() -> PushQueuesBuilder<F> { PushQueuesBuilder::default() }

    pub(super) fn build_with_rate_dlq(
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
        };
        Ok((
            Self {
                high_priority_rx: high_rx,
                low_priority_rx: low_rx,
            },
            PushHandle::from_arc(Arc::new(inner)),
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
    /// than [`MAX_PUSH_RATE`] and [`PushConfigError::InvalidCapacity`] if either
    /// queue capacity is zero.
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
    /// than [`MAX_PUSH_RATE`] and [`PushConfigError::InvalidCapacity`] if either
    /// queue capacity is zero.
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
    ///         .expect("failed to build push queues");
    ///     handle.push_high_priority(2u8).await.expect("push failed");
    ///     let (priority, frame) = queues.recv().await.expect("recv failed");
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
    /// let (mut queues, _handle) = PushQueues::<u8>::builder()
    ///     .build()
    ///     .expect("failed to build push queues");
    /// queues.close();
    /// ```
    pub fn close(&mut self) {
        self.high_priority_rx.close();
        self.low_priority_rx.close();
    }
}
