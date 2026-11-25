//! Coverage for the counting window's boundaries.
//!
//! Unit-test builds do not install `INSTRUMENTED_SYSTEM`, so the counter here
//! stays at zero whatever runs inside a window. These tests therefore cover
//! only what is observable without the allocator: that the gate opens for the
//! operation and closes again on the success, error, and panic paths. The
//! counting itself is checked where the allocator is actually installed —
//! `benches/runtime_ownership_baselines_alloc/probes.rs` runs a zero-window and
//! a positive-window self-check before measuring, so a gate that never opens is
//! a loud failure rather than a silent baseline of zeros.
//!
//! Both tests are `serial`: the flag they assert on is process-global, so
//! running them concurrently lets one test's open window be read by the other's
//! closing assertion. That race made
//! [`counting_window_closes_after_success`](counting_window_closes_after_success)
//! fail roughly one run in five even in isolation, which is a flake in the test
//! rather than a defect in the window.

use serial_test::serial;

use super::{allocation_count, count_allocations, is_allocation_counting_enabled};

#[test]
#[serial]
fn counting_window_closes_after_success() {
    let _count = count_allocations(|| Ok(())).expect("no-op measurement must succeed");
    assert!(
        !is_allocation_counting_enabled(),
        "a successful measurement must close the window"
    );
}

#[test]
#[serial]
fn counting_window_closes_after_failure() {
    let error = count_allocations(|| Err("deliberate failure".to_owned()))
        .expect_err("a failing operation must surface its error");
    assert_eq!(
        error, "deliberate failure",
        "error should equal \"deliberate failure\""
    );
    assert!(
        !is_allocation_counting_enabled(),
        "a failed measurement must still close the window"
    );
}

#[test]
#[serial]
fn counting_window_closes_after_panic() {
    // A measured operation may panic: an assertion inside a benchmark harness,
    // a failed `expect`, or a formatter that cannot render the value. The
    // window is process-global, so leaving the gate open would silently
    // attribute every later allocation to the abandoned window.
    let panicked = std::panic::catch_unwind(|| {
        let _count = count_allocations(|| panic!("deliberate panic inside the window"));
    });
    assert!(panicked.is_err(), "a panicking operation must unwind");
    assert!(
        !is_allocation_counting_enabled(),
        "a panicking measurement must still close the window"
    );
}

#[test]
#[serial]
fn empty_window_counts_nothing() {
    let count = count_allocations(|| Ok(())).expect("no-op measurement must succeed");
    assert_eq!(count, 0, "an empty window must report exactly zero");
    assert_eq!(
        allocation_count(),
        0,
        "the accumulated count must match an empty window"
    );
}
