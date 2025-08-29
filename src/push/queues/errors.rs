//! Error types for push queue operations and configuration.

use std::fmt;

/// Errors that can occur when pushing a frame.
#[derive(Debug)]
pub enum PushError {
    /// The queue was at capacity and the policy was `ReturnErrorIfFull`.
    QueueFull,
    /// The receiving end of the queue has been dropped.
    Closed,
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    /// The provided rate was zero or exceeded [`super::MAX_PUSH_RATE`].
    InvalidRate(usize),
}

impl fmt::Display for PushConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRate(r) => {
                write!(
                    f,
                    "invalid rate {r}; must be between 1 and {}",
                    super::MAX_PUSH_RATE
                )
            }
        }
    }
}

impl std::error::Error for PushConfigError {}
