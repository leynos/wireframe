//! Builder for configuring push queues.

use std::time::Duration;

use tokio::sync::mpsc;

use super::{
    DEFAULT_PUSH_RATE,
    FrameLike,
    FrameLimit,
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
/// a capacity of one and pushes are limited to the default push rate per
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
/// Pending queue and dead-letter settings consumed by `build`.
pub struct PushQueuesBuilder<F> {
    /// Buffer size for urgent frames.
    high_capacity: usize,
    /// Buffer size for best-effort frames.
    low_capacity: usize,
    /// Optional global pushes-per-second limiter.
    rate: Option<usize>,
    /// Optional sink receiving frames dropped under a full-queue policy.
    dlq: Option<mpsc::Sender<F>>,
    /// Number of drops between count-based diagnostics.
    dlq_log_every_n: usize,
    /// Minimum interval between drop diagnostics.
    dlq_log_interval: Duration,
    /// Optional payload-size limit applied before a frame is queued.
    max_frame_len: Option<FrameLimit<F>>,
}

// Written by hand so a stored builder can seed per-connection queues without
// requiring the frame type to be `Clone`. The dead-letter sender is cloneable
// independently of its payload; every clone shares the same sink.
impl<F> Clone for PushQueuesBuilder<F> {
    fn clone(&self) -> Self {
        Self {
            high_capacity: self.high_capacity,
            low_capacity: self.low_capacity,
            rate: self.rate,
            dlq: self.dlq.clone(),
            dlq_log_every_n: self.dlq_log_every_n,
            dlq_log_interval: self.dlq_log_interval,
            max_frame_len: self.max_frame_len,
        }
    }
}

impl<F: FrameLike> Default for PushQueuesBuilder<F> {
    fn default() -> Self {
        Self {
            high_capacity: 1,
            low_capacity: 1,
            rate: Some(DEFAULT_PUSH_RATE),
            dlq: None,
            // Default to logging every DLQ drop for visibility in tests and
            // development. Can be adjusted by applications that need lower
            // verbosity.
            dlq_log_every_n: 1,
            dlq_log_interval: Duration::from_secs(10),
            max_frame_len: None,
        }
    }
}

impl<F: FrameLike> PushQueuesBuilder<F> {
    /// Set the capacity of the high-priority queue.
    #[must_use]
    pub const fn high_capacity(mut self, capacity: usize) -> Self {
        self.high_capacity = capacity;
        self
    }

    /// Set the capacity of the low-priority queue.
    #[must_use]
    pub const fn low_capacity(mut self, capacity: usize) -> Self {
        self.low_capacity = capacity;
        self
    }

    /// Set the global push rate limit in pushes per second.
    ///
    /// Passing `None` disables rate limiting.
    #[must_use]
    pub const fn rate(mut self, rate: Option<usize>) -> Self {
        self.rate = rate;
        self
    }

    /// Disable rate limiting entirely.
    #[must_use]
    pub const fn unlimited(self) -> Self { self.rate(None) }

    /// Provide a dead-letter queue for discarded frames.
    ///
    /// Frames are dropped when no DLQ is set or the channel is full.
    #[must_use]
    pub fn dlq(mut self, dlq: Option<mpsc::Sender<F>>) -> Self {
        self.dlq = dlq;
        self
    }

    /// Log dropped frames every `n` occurrences when the DLQ is full or closed.
    /// A value of `0` disables count-based logging; interval-based logging
    /// still applies.
    #[must_use]
    pub const fn dlq_log_every_n(mut self, n: usize) -> Self {
        // Treat 0 as “disabled by count”; interval-based logging remains.
        self.dlq_log_every_n = n;
        self
    }

    /// Reject pushes whose encoded payload exceeds `max` bytes.
    ///
    /// `payload_len` reports how many payload bytes a frame encodes to. It is
    /// supplied by the codec rather than assumed, because a codec's frame type
    /// need not expose its own payload length. The check runs before a frame is
    /// queued, so a push the connection cannot encode is reported to its
    /// producer instead of failing the connection when the encoder sees it.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::push::PushQueues;
    ///
    /// # fn main() {
    /// let (_queues, _handle) = PushQueues::<Vec<u8>>::builder()
    ///     .max_frame_len(64, |frame: &Vec<u8>| frame.len())
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// # }
    /// ```
    #[must_use]
    pub fn max_frame_len(mut self, max: usize, payload_len: fn(&F) -> usize) -> Self {
        self.max_frame_len = Some(FrameLimit { max, payload_len });
        self
    }

    /// Apply a codec's frame ceiling without relaxing a stricter limit.
    ///
    /// The connection writes the codec's own frames, so the codec supplies the
    /// payload measure either way; only the bound is clamped. A template that
    /// already limits frames more tightly than its codec does keeps its own
    /// bound, so configuring a codec ceiling never widens one already chosen.
    #[must_use]
    pub(crate) fn capped_by_codec(mut self, max: usize, payload_len: fn(&F) -> usize) -> Self {
        let limit = self
            .max_frame_len
            .map_or(max, |existing| existing.max.min(max));
        self.max_frame_len = Some(FrameLimit {
            max: limit,
            payload_len,
        });
        self
    }

    /// Minimum interval between DLQ drop log entries.
    #[must_use]
    pub const fn dlq_log_interval(mut self, interval: Duration) -> Self {
        self.dlq_log_interval = interval;
        self
    }

    /// Validate the stored configuration without allocating its channels.
    ///
    /// This lets a stored template be rejected while preparing an application,
    /// before any connection exists. [`Self::build`] uses the same checks.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if the rate is zero or strictly
    /// greater than [`super::MAX_PUSH_RATE`] and
    /// [`PushConfigError::InvalidCapacity`] if either queue capacity is zero or
    /// greater than [`tokio::sync::Semaphore::MAX_PERMITS`].
    pub(crate) const fn validate(&self) -> Result<(), PushConfigError> {
        if let Some(invalid) = PushQueues::<F>::invalid_rate(self.rate) {
            return Err(PushConfigError::InvalidRate(invalid));
        }
        if let Some((high, low)) =
            PushQueues::<F>::invalid_capacity(self.high_capacity, self.low_capacity)
        {
            return Err(PushConfigError::InvalidCapacity { high, low });
        }
        Ok(())
    }

    /// Build the configured [`PushQueues`] and associated [`PushHandle`].
    ///
    /// Validation of capacities and rate occurs only during the build step,
    /// not when setting individual fields. A `dlq_log_every_n` of `0` disables
    /// count-based logging; interval-based logging still applies.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError::InvalidRate`] if the rate is zero or strictly
    /// greater than [`super::MAX_PUSH_RATE`] and
    /// [`PushConfigError::InvalidCapacity`] if either queue capacity is zero or
    /// greater than [`tokio::sync::Semaphore::MAX_PERMITS`].
    pub fn build(self) -> Result<(PushQueues<F>, PushHandle<F>), PushConfigError> {
        self.validate()?;
        let config = PushQueueConfig {
            high_capacity: self.high_capacity,
            low_capacity: self.low_capacity,
            rate: self.rate,
            dlq: self.dlq,
            dlq_log_every_n: self.dlq_log_every_n,
            dlq_log_interval: self.dlq_log_interval,
        };
        PushQueues::build_with_config(config, self.max_frame_len)
    }
}
