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
/// ```plaintext
/// # HELP wireframe_connection_panics_total Count of panicking connection tasks.
/// # TYPE wireframe_connection_panics_total counter
/// wireframe_connection_panics_total 1
/// ```
pub const CONNECTION_PANICS: &str = "wireframe_connection_panics_total";
/// Name of the counter tracking recovered client pool bookkeeping lock poison.
///
/// ```plaintext
/// # HELP wireframe_pool_bookkeeping_poison_recoveries_total Count of recovered client pool bookkeeping locks.
/// # TYPE wireframe_pool_bookkeeping_poison_recoveries_total counter
/// wireframe_pool_bookkeeping_poison_recoveries_total 1
/// ```
pub const POOL_BOOKKEEPING_POISON_RECOVERIES: &str =
    "wireframe_pool_bookkeeping_poison_recoveries_total";

/// Name of the counter tracking codec errors by type and recovery policy.
///
/// ```plaintext
/// # HELP wireframe_codec_errors_total Count of codec errors by type and recovery policy.
/// # TYPE wireframe_codec_errors_total counter
/// wireframe_codec_errors_total{error_type="framing",recovery_policy="drop"} 5
/// wireframe_codec_errors_total{error_type="eof",recovery_policy="disconnect"} 2
/// ```
pub const CODEC_ERRORS: &str = "wireframe_codec_errors_total";

/// Name of the counter tracking server-supervisor cancellation requests.
///
/// The `reason` label is always either `"graceful"`, when the supplied
/// shutdown future resolved, or `"dropped"`, when the supervisor future was
/// dropped before graceful shutdown completed. The latter includes task abort.
///
/// ```plaintext
/// # HELP wireframe_server_supervisor_cancellations_total Count of server-supervisor cancellation requests.
/// # TYPE wireframe_server_supervisor_cancellations_total counter
/// wireframe_server_supervisor_cancellations_total{reason="graceful"} 1
/// ```
pub const SERVER_SUPERVISOR_CANCELLATIONS: &str = "wireframe_server_supervisor_cancellations_total";

/// Name of the counter tracking accept loops that observed cancellation.
///
/// The `reason` label has the same bounded vocabulary as
/// [`SERVER_SUPERVISOR_CANCELLATIONS`].
///
/// ```plaintext
/// # HELP wireframe_server_accept_loops_exited_total Count of accept loops that exited after cancellation.
/// # TYPE wireframe_server_accept_loops_exited_total counter
/// wireframe_server_accept_loops_exited_total{reason="dropped"} 2
/// ```
pub const SERVER_ACCEPT_LOOPS_EXITED: &str = "wireframe_server_accept_loops_exited_total";

/// Bounded reasons for server-supervisor cancellation metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerCancellationReason {
    /// Cancellation followed completion of the graceful-shutdown future.
    Graceful,
    /// Cancellation followed the server-supervisor future being dropped.
    Dropped,
}

impl ServerCancellationReason {
    /// Return the stable metric and tracing label value for this reason.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Dropped => "dropped",
        }
    }
}

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

/// Increment the active connections gauge.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn inc_connections() {}

/// Decrement the active connections gauge.
#[cfg(feature = "metrics")]
pub fn dec_connections() { gauge!(CONNECTIONS_ACTIVE).decrement(1.0); }

/// Decrement the active connections gauge.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn dec_connections() {}

/// Record a processed frame for the given direction.
#[cfg(feature = "metrics")]
pub fn inc_frames(direction: Direction) {
    counter!(FRAMES_PROCESSED, "direction" => direction.as_str()).increment(1);
}

/// Record a processed frame for the given direction.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn inc_frames(_direction: Direction) {}

/// Record a deserialization error.
#[cfg(feature = "metrics")]
pub fn inc_deser_errors() { counter!(ERRORS_TOTAL, "kind" => "deserialization").increment(1); }

/// Record a deserialization error.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn inc_deser_errors() {}

/// Record a handler error.
#[cfg(feature = "metrics")]
pub fn inc_handler_errors() { counter!(ERRORS_TOTAL, "kind" => "handler").increment(1); }

/// Record a handler error.
///
/// This function is a no-op when the `metrics` feature is disabled.
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

/// Record a panicking connection task.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn inc_connection_panics() {}

/// Record a recovered client pool bookkeeping lock poison event.
#[cfg(feature = "metrics")]
pub fn inc_pool_bookkeeping_poison_recoveries() {
    counter!(POOL_BOOKKEEPING_POISON_RECOVERIES).increment(1);
}

/// Record a recovered client pool bookkeeping lock poison event.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn inc_pool_bookkeeping_poison_recoveries() {}

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

/// Record a codec error with its type and recovery policy.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn inc_codec_error(_error_type: &'static str, _recovery_policy: &'static str) {}

/// Record a server-supervisor cancellation request with a bounded reason.
#[cfg(feature = "metrics")]
pub(crate) fn inc_server_supervisor_cancellation(reason: ServerCancellationReason) {
    counter!(SERVER_SUPERVISOR_CANCELLATIONS, "reason" => reason.as_str()).increment(1);
}

/// Record a server-supervisor cancellation request with a bounded reason.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_server_supervisor_cancellation(_reason: ServerCancellationReason) {}

/// Record an accept loop that exited after supervisor cancellation.
#[cfg(feature = "metrics")]
pub(crate) fn inc_server_accept_loop_exit(reason: ServerCancellationReason) {
    counter!(SERVER_ACCEPT_LOOPS_EXITED, "reason" => reason.as_str()).increment(1);
}

/// Record an accept loop that exited after supervisor cancellation.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_server_accept_loop_exit(_reason: ServerCancellationReason) {}
