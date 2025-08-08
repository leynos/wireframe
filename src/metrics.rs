//! Metric helpers for `wireframe`.
//!
//! This module defines metric names and helper functions wrapping the
//! [`metrics`](https://docs.rs/metrics) crate. All functions become no-ops
//! if the optional `metrics` Cargo feature is disabled.
//!
//! # Prometheus Integration
//!
//! ```
//! use metrics_exporter_prometheus::PrometheusBuilder;
//!
//! let handle = PrometheusBuilder::new()
//!     .install_recorder()
//!     .expect("recorder install");
//! println!("{}", handle.render());
//! ```

use std::net::SocketAddr;

#[cfg(feature = "metrics")]
use metrics::{counter, gauge};

/// Name of the gauge tracking active connections.
pub const CONNECTIONS_ACTIVE: &str = "wireframe_connections_active";
/// Name of the counter tracking processed frames.
pub const FRAMES_PROCESSED: &str = "wireframe_frames_processed_total";
/// Name of the counter tracking error occurrences.
pub const ERRORS_TOTAL: &str = "wireframe_errors_total";
/// Name of the counter tracking connection panics.
pub const CONNECTION_PANICS: &str = "wireframe_connection_panics_total";

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
#[cfg(feature = "metrics")]
pub fn inc_connections() { gauge!(CONNECTIONS_ACTIVE).increment(1.0); }

#[cfg(not(feature = "metrics"))]
pub fn inc_connections() {}

/// Decrement the active connections gauge.
#[cfg(feature = "metrics")]
pub fn dec_connections() { gauge!(CONNECTIONS_ACTIVE).decrement(1.0); }

#[cfg(not(feature = "metrics"))]
pub fn dec_connections() {}

/// Record a processed frame for the given direction.
#[cfg(feature = "metrics")]
pub fn inc_frames(direction: Direction) {
    counter!(FRAMES_PROCESSED, "direction" => direction.as_str()).increment(1);
}

#[cfg(not(feature = "metrics"))]
pub fn inc_frames(_direction: Direction) {}

/// Record a deserialization error.
#[cfg(feature = "metrics")]
pub fn inc_deser_errors() { counter!(ERRORS_TOTAL, "kind" => "deserialization").increment(1); }

#[cfg(not(feature = "metrics"))]
pub fn inc_deser_errors() {}

/// Record a handler error.
#[cfg(feature = "metrics")]
pub fn inc_handler_errors() { counter!(ERRORS_TOTAL, "kind" => "handler").increment(1); }

#[cfg(not(feature = "metrics"))]
pub fn inc_handler_errors() {}

/// Record a panicking connection task.
#[cfg(feature = "metrics")]
pub fn inc_connection_panics(peer_addr: Option<SocketAddr>) {
    match peer_addr {
        Some(addr) => {
            counter!(CONNECTION_PANICS, "peer_addr" => addr.to_string()).increment(1);
        }
        None => {
            counter!(CONNECTION_PANICS, "peer_addr" => "unknown").increment(1);
        }
    }
}

#[cfg(not(feature = "metrics"))]
pub fn inc_connection_panics(_peer_addr: Option<SocketAddr>) {}
