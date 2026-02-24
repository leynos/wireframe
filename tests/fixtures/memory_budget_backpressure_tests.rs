//! Unit tests for `BackpressureConfig` parsing.

use std::str::FromStr;

use super::memory_budget_backpressure::BackpressureConfig;

#[test]
fn parses_valid_backpressure_config() {
    let config = BackpressureConfig::from_str("1000/10/20/30").expect("valid config should parse");

    assert_eq!(config.timeout_ms, 1000);
    assert_eq!(config.per_message, 10);
    assert_eq!(config.per_connection, 20);
    assert_eq!(config.in_flight, 30);
}

#[test]
fn fails_when_missing_fields() {
    let missing_in_flight =
        BackpressureConfig::from_str("1000/10/20").expect_err("missing in_flight should fail");
    assert_eq!(missing_in_flight, "missing in_flight");

    let missing_per_message =
        BackpressureConfig::from_str("1000").expect_err("missing per_message should fail");
    assert_eq!(missing_per_message, "missing per_message");

    let missing_timeout =
        BackpressureConfig::from_str("").expect_err("missing timeout_ms should fail");
    assert_eq!(missing_timeout, "missing timeout_ms");
}

#[test]
fn fails_with_clear_error_on_non_numeric_segments() {
    let timeout_error = BackpressureConfig::from_str("not-a-number/10/20/30")
        .expect_err("non-numeric timeout_ms should fail");
    assert!(
        timeout_error.starts_with("timeout_ms:"),
        "unexpected error message: {timeout_error}"
    );

    let per_message_error = BackpressureConfig::from_str("1000/not-a-number/20/30")
        .expect_err("non-numeric per_message should fail");
    assert!(
        per_message_error.starts_with("per_message:"),
        "unexpected error message: {per_message_error}"
    );

    let per_connection_error = BackpressureConfig::from_str("1000/10/not-a-number/30")
        .expect_err("non-numeric per_connection should fail");
    assert!(
        per_connection_error.starts_with("per_connection:"),
        "unexpected error message: {per_connection_error}"
    );

    let in_flight_error = BackpressureConfig::from_str("1000/10/20/not-a-number")
        .expect_err("non-numeric in_flight should fail");
    assert!(
        in_flight_error.starts_with("in_flight:"),
        "unexpected error message: {in_flight_error}"
    );
}

#[test]
fn fails_when_segments_are_present_after_in_flight() {
    let trailing_segments = BackpressureConfig::from_str("1000/10/20/30/40")
        .expect_err("trailing segments should fail");
    assert_eq!(trailing_segments, "unexpected trailing segments");
}
