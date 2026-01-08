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

#[cfg(feature = "metrics")]
use metrics::{counter, gauge};

/// Name of the gauge tracking active connections.
pub const CONNECTIONS_ACTIVE: &str = "wireframe_connections_active";
/// Name of the counter tracking processed frames.
pub const FRAMES_PROCESSED: &str = "wireframe_frames_processed_total";
/// Name of the counter tracking error occurrences.
pub const ERRORS_TOTAL: &str = "wireframe_errors_total";
/// Name of the counter tracking connection panics.
///
/// ```text
/// # HELP wireframe_connection_panics_total Count of panicking connection tasks.
/// # TYPE wireframe_connection_panics_total counter
/// wireframe_connection_panics_total 1
/// ```
pub const CONNECTION_PANICS: &str = "wireframe_connection_panics_total";

/// Name of the counter tracking codec errors by type and recovery policy.
///
/// ```text
/// # HELP wireframe_codec_errors_total Count of codec errors by type and recovery policy.
/// # TYPE wireframe_codec_errors_total counter
/// wireframe_codec_errors_total{error_type="framing",recovery_policy="drop"} 5
/// wireframe_codec_errors_total{error_type="eof",recovery_policy="disconnect"} 2
/// ```
pub const CODEC_ERRORS: &str = "wireframe_codec_errors_total";

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
///
/// # Examples
///
/// ```no_run
/// use std::panic::catch_unwind;
///
/// use wireframe::metrics;
///
/// let res = catch_unwind(|| {
///     panic!("boom");
/// });
/// if res.is_err() {
///     metrics::inc_connection_panics();
/// }
/// ```
#[cfg(feature = "metrics")]
pub fn inc_connection_panics() { counter!(CONNECTION_PANICS).increment(1); }

#[cfg(not(feature = "metrics"))]
pub fn inc_connection_panics() {}

/// Record a codec error with its type and recovery policy.
///
/// # Arguments
///
/// * `error_type` - The category of error: `"framing"`, `"protocol"`, `"io"`, or `"eof"`.
/// * `recovery_policy` - The recovery action taken: `"drop"`, `"quarantine"`, or `"disconnect"`.
///
/// # Examples
///
/// ```no_run
/// use wireframe::metrics;
///
/// // Record an oversized frame that was dropped
/// metrics::inc_codec_error("framing", "drop");
///
/// // Record a premature EOF that caused disconnect
/// metrics::inc_codec_error("eof", "disconnect");
/// ```
#[cfg(feature = "metrics")]
pub fn inc_codec_error(error_type: &'static str, recovery_policy: &'static str) {
    counter!(
        CODEC_ERRORS,
        "error_type" => error_type,
        "recovery_policy" => recovery_policy
    )
    .increment(1);
}

#[cfg(not(feature = "metrics"))]
pub fn inc_codec_error(_error_type: &'static str, _recovery_policy: &'static str) {}
