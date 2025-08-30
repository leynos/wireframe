//! Error types for push queue operations and configuration.

use thiserror::Error;

use super::MAX_PUSH_RATE;

/// Errors that can occur when pushing a frame.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum PushError {
    /// The queue was at capacity and the policy was `ReturnErrorIfFull`.
    #[error("push queue full")]
    QueueFull,
    /// The receiving end of the queue has been dropped.
    #[error("push queue closed")]
    Closed,
}

/// Errors returned when creating push queues.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum PushConfigError {
    /// The provided rate was zero or exceeded [`MAX_PUSH_RATE`].
    #[error("invalid rate {0}; must be between 1 and {max}", max = MAX_PUSH_RATE)]
    InvalidRate(usize),
    /// The provided capacities were zero.
    #[error("invalid capacities; high={high}, low={low}; each must be ≥ 1")]
    InvalidCapacity { high: usize, low: usize },
}
