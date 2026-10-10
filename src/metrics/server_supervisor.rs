//! Abnormal server-supervisor termination metrics.

#[cfg(feature = "metrics")]
use ::metrics::counter;

/// Name of the counter tracking abnormal server-supervisor terminations.
///
/// ```plaintext
/// # HELP wireframe_server_supervisor_abnormal_terminations_total Count of abnormal server-supervisor terminations.
/// # TYPE wireframe_server_supervisor_abnormal_terminations_total counter
/// wireframe_server_supervisor_abnormal_terminations_total 1
/// ```
pub const SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS: &str =
    "wireframe_server_supervisor_abnormal_terminations_total";

/// Record an abnormal server-supervisor termination.
#[cfg(feature = "metrics")]
pub(crate) fn inc_server_supervisor_abnormal_termination() {
    counter!(SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS).increment(1);
}

/// Record an abnormal server-supervisor termination.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_server_supervisor_abnormal_termination() {}
