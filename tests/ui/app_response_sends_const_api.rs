//! Verifies the public app-response send metric name is usable in const
//! contexts.

use wireframe::metrics::APP_RESPONSE_SENDS;

/// The counter name must be a `&'static str` constant, not a runtime lookup.
const METRIC_NAME: &str = APP_RESPONSE_SENDS;

fn main() {
    assert_eq!(METRIC_NAME, "wireframe_app_response_sends_total");
}