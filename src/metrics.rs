//! Metric helpers for `wireframe`.
//!
//! This module defines metric names and simple helper functions
//! wrapping the [`metrics`](https://docs.rs/metrics) crate.

use metrics::{counter, gauge};

/// Name of the gauge tracking active connections.
pub const CONNECTIONS_ACTIVE: &str = "wireframe_connections_active";
/// Name of the counter tracking processed frames.
pub const FRAMES_PROCESSED: &str = "wireframe_frames_processed_total";
/// Name of the counter tracking error occurrences.
pub const ERRORS_TOTAL: &str = "wireframe_errors_total";

/// Direction of frame processing.
#[derive(Clone, Copy)]
pub enum Direction {
    /// Inbound frames received from a client.
    Inbound,
    /// Outbound frames sent to a client.
    Outbound,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Inbound => "inbound",
            Direction::Outbound => "outbound",
        }
    }
}

/// Increment the active connections gauge.
pub fn inc_connections() { gauge!(CONNECTIONS_ACTIVE).increment(1.0); }

/// Decrement the active connections gauge.
pub fn dec_connections() { gauge!(CONNECTIONS_ACTIVE).decrement(1.0); }

/// Record a processed frame for the given direction.
pub fn inc_frames(direction: Direction) {
    counter!(FRAMES_PROCESSED, "direction" => direction.as_str()).increment(1);
}

/// Record an error occurrence.
pub fn inc_errors() { counter!(ERRORS_TOTAL).increment(1); }
