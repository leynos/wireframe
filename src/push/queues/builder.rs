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
/// ```
/// use wireframe::push::PushQueues;
///
/// let (_queues, _handle) = PushQueues::<u8>::builder()
///     .build()
///     .expect("failed to build push queues");
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
    /// [`super::MAX_PUSH_RATE`] and [`PushConfigError::InvalidCapacity`] if
    /// either queue capacity is zero.
    pub fn build(self) -> Result<(PushQueues<F>, PushHandle<F>), PushConfigError> {
        PushQueues::build_with_rate_dlq(self.high_capacity, self.low_capacity, self.rate, self.dlq)
    }
}
