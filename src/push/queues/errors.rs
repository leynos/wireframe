//! Error types for push queue operations and configuration.

use thiserror::Error;

use super::MAX_PUSH_RATE;

/// Errors that can occur when pushing a frame.
#[non_exhaustive]
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    /// The queue was at capacity and the policy was `ReturnErrorIfFull`.
    #[error("push queue full")]
    QueueFull,
    /// The receiving end of the queue has been dropped.
    #[error("push queue closed")]
    Closed,
    /// The connection's rate limiter had no permit available.
    ///
    /// Returned only by the non-blocking
    /// [`PushHandle::try_push`](crate::push::PushHandle::try_push), which never
    /// waits. The awaited push helpers wait for the next refill instead.
    #[error("push rate limit reached")]
    RateLimited,
    /// The frame's payload exceeds the connection codec's frame limit.
    #[error("push frame of {size} bytes exceeds the {max}-byte frame limit")]
    FrameTooLarge {
        /// Encoded payload size of the rejected frame.
        size: usize,
        /// Largest payload the connection codec will encode.
        max: usize,
    },
}

/// Errors returned when creating push queues.
#[non_exhaustive]
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PushConfigError {
    /// The provided rate was zero or exceeded [`crate::push::queues::MAX_PUSH_RATE`].
    #[error("invalid rate {0}; must be between 1 and {max}", max = MAX_PUSH_RATE)]
    InvalidRate(usize),
    /// The provided capacities were zero or exceeded the channel limit.
    #[error(
        "invalid capacities; high={high}, low={low}; each must be between 1 and {max}",
        max = tokio::sync::Semaphore::MAX_PERMITS
    )]
    InvalidCapacity {
        /// Capacity configured for the high-priority queue.
        high: usize,
        /// Capacity configured for the low-priority queue.
        low: usize,
    },
}
