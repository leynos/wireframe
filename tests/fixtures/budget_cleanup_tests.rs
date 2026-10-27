//! Fixture self-tests for budget cleanup world wiring and parsing.

use std::io;

use super::BudgetCleanupWorld;

#[test]
fn record_send_result_clears_last_error_after_success() {
    let mut world = BudgetCleanupWorld::default();
    let first_failure = io::Error::other("first failure");
    let first_result = world.record_send_result(Ok(Err(first_failure)));
    let first_error = first_result.expect_err("first send should fail");
    assert_eq!(first_error.to_string(), "io error: first failure");
    assert_eq!(
        world.last_send_error.as_deref(),
        Some("io error: first failure")
    );

    let success_result = world.record_send_result(Ok(Ok(())));
    assert!(success_result.is_ok(), "second send should succeed");
    assert_eq!(world.last_send_error, None);
}

#[test]
fn record_send_result_stores_outer_error_string() {
    let mut world = BudgetCleanupWorld::default();
    let outer_error = io::Error::other("outer failure");

    let result = world.record_send_result(Err(outer_error.into()));
    let returned_error = result.expect_err("outer error should be returned");
    assert_eq!(returned_error.to_string(), "io error: outer failure");
    assert_eq!(
        world.last_send_error.as_deref(),
        Some("io error: outer failure")
    );
}

#[test]
fn record_send_result_converts_inner_io_error_and_stores_it() {
    let mut world = BudgetCleanupWorld::default();
    let result = world.record_send_result(Ok(Err(io::Error::other("inner io failure"))));
    let returned_error = result.expect_err("inner IO error should be returned");
    assert_eq!(returned_error.to_string(), "io error: inner io failure");
    assert_eq!(
        world.last_send_error.as_deref(),
        Some("io error: inner io failure")
    );
}
