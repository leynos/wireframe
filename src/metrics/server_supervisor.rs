//! Abnormal server-supervisor termination metrics.

#[cfg(all(feature = "metrics", not(loom)))]
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
#[cfg(all(feature = "metrics", not(loom)))]
pub(crate) fn inc_server_supervisor_abnormal_termination() {
    counter!(SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS).increment(1);
}

/// Record an abnormal server-supervisor termination.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), not(loom)))]
pub(crate) fn inc_server_supervisor_abnormal_termination() {}
