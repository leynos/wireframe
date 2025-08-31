//! Builder for configuring push queues.

use std::time::Duration;

use tokio::sync::mpsc;

use super::{
    DEFAULT_PUSH_RATE,
    FrameLike,
    PushConfigError,
    PushHandle,
    PushQueueConfig,
    PushQueues,
};

/// Builder for [`PushQueues`].
///
/// Allows configuration of queue capacities, rate limiting and an optional
/// dead-letter queue before constructing [`PushQueues`] and its paired
/// [`PushHandle`]. Defaults mirror the previous constructors: both queues have
/// a capacity of one and pushes are limited to [`DEFAULT_PUSH_RATE`] per
/// second unless overridden. Construct via [`PushQueues::builder`] or
/// [`Default::default`].
///
/// # Examples
///
/// Build queues with custom capacities, a rate limit and a dead-letter queue:
///
/// ```rust,no_run
/// use tokio::sync::mpsc;
/// use wireframe::push::PushQueues;
///
/// # async fn demo() {
/// let (dlq_tx, _dlq_rx) = mpsc::channel(8);
/// let (_queues, _handle) = PushQueues::<u8>::builder()
///     .high_capacity(8)
///     .low_capacity(8)
///     .rate(Some(100))
///     .dlq(Some(dlq_tx)) // frames are dropped if no DLQ or DLQ is full
///     .build()
///     .expect("failed to build PushQueues");
/// # }
/// ```
///
/// Disable rate limiting with the `unlimited` convenience:
///
/// ```rust,no_run
/// use wireframe::push::PushQueues;
/// let (_queues, _handle) = PushQueues::<u8>::builder()
///     .unlimited()
///     .build()
///     .expect("failed to build PushQueues");
/// # drop((_queues, _handle));
/// ```
///
/// Builders can also be constructed directly:
///
/// ```
/// use wireframe::push::PushQueuesBuilder;
///
/// let (_queues, _handle) = PushQueuesBuilder::<u8>::default()
///     .build()
///     .expect("failed to build PushQueues");
/// # drop((_queues, _handle));
/// ```
#[derive(Debug)]
pub struct PushQueuesBuilder<F> {
    high_capacity: usize,
    low_capacity: usize,
    rate: Option<usize>,
    dlq: Option<mpsc::Sender<F>>,
    dlq_log_every_n: usize,
    dlq_log_interval: Duration,
}

impl<F: FrameLike> Default for PushQueuesBuilder<F> {
    fn default() -> Self {
        Self {
            high_capacity: 1,
            low_capacity: 1,
            rate: Some(DEFAULT_PUSH_RATE),
            dlq: None,
            dlq_log_every_n: 100,
            dlq_log_interval: Duration::from_secs(10),
        }
    }
}

impl<F: FrameLike> PushQueuesBuilder<F> {
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
    ///
    /// Passing `None` disables rate limiting.
    #[must_use]
    pub fn rate(mut self, rate: Option<usize>) -> Self {
        self.rate = rate;
        self
    }

    /// Disable rate limiting entirely.
    #[must_use]
    pub fn unlimited(self) -> Self { self.rate(None) }

    /// Provide a dead-letter queue for discarded frames.
    ///
    /// Frames are dropped when no DLQ is set or the channel is full.
    #[must_use]
    pub fn dlq(mut self, dlq: Option<mpsc::Sender<F>>) -> Self {
        self.dlq = dlq;
        self
    }

    /// Log dropped frames every `n` occurrences when the DLQ is full or closed.
    #[must_use]
    pub fn dlq_log_every_n(mut self, n: usize) -> Self {
        self.dlq_log_every_n = n;
        self
    }

    /// Minimum interval between DLQ drop log entries.
    #[must_use]
    pub fn dlq_log_interval(mut self, interval: Duration) -> Self {
        self.dlq_log_interval = interval;
        self
    }

    /// Build the configured [`PushQueues`] and associated [`PushHandle`].
    ///
    /// Validation of capacities and rate occurs only during the build step,
    /// not when setting individual fields.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if the rate is zero or strictly
    /// greater than [`super::MAX_PUSH_RATE`] and
    /// [`PushConfigError::InvalidCapacity`] if either queue capacity is zero.
    pub fn build(self) -> Result<(PushQueues<F>, PushHandle<F>), PushConfigError> {
        PushQueues::build_with_config(PushQueueConfig {
            high_capacity: self.high_capacity,
            low_capacity: self.low_capacity,
            rate: self.rate,
            dlq: self.dlq,
            dlq_log_every_n: self.dlq_log_every_n,
            dlq_log_interval: self.dlq_log_interval,
        })
    }
}
