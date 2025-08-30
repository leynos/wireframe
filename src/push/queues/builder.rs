//! Builder for configuring push queues.

use tokio::sync::mpsc;

use super::{DEFAULT_PUSH_RATE, FrameLike, PushConfigError, PushHandle, PushQueues};

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
///     .rate(Some(100)) // pass None to disable rate limiting
///     .dlq(Some(dlq_tx)) // frames are dropped if no DLQ or DLQ is full
///     .build()
///     .expect("failed to build PushQueues");
/// # }
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
}

impl<F: FrameLike> Default for PushQueuesBuilder<F> {
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
    /// Set the capacity of the high-priority queue.
    #[must_use]
    pub fn high_capacity(mut self, capacity: usize) -> Self {
        debug_assert!(capacity > 0, "capacity must be greater than zero");
        self.high_capacity = capacity;
        self
    }

    /// Set the capacity of the low-priority queue.
    #[must_use]
    pub fn low_capacity(mut self, capacity: usize) -> Self {
        debug_assert!(capacity > 0, "capacity must be greater than zero");
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

    /// Provide a dead-letter queue for discarded frames.
    ///
    /// Frames are dropped when no DLQ is set or the channel is full.
    #[must_use]
    pub fn dlq(mut self, dlq: Option<mpsc::Sender<F>>) -> Self {
        self.dlq = dlq;
        self
    }

    /// Build the configured [`PushQueues`] and associated [`PushHandle`].
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if the rate is zero or strictly
    /// greater than [`super::MAX_PUSH_RATE`] and
    /// [`PushConfigError::InvalidCapacity`] if either queue capacity is zero.
    pub fn build(self) -> Result<(PushQueues<F>, PushHandle<F>), PushConfigError> {
        PushQueues::build_with_rate_dlq(self.high_capacity, self.low_capacity, self.rate, self.dlq)
    }
}
