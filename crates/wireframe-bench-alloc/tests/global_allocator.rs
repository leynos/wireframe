//! Proof that the counting allocator is driven by a real process allocator.
//!
//! The crate's unit tests call the `GlobalAlloc` methods directly, which proves
//! each delegation in isolation but never shows that the runtime routes ordinary
//! allocations through this type. This binary installs the allocator as its
//! `#[global_allocator]`, so every allocation the test and the standard library
//! make flows through it, and the counting window is observed end to end.
//!
//! This is also the target Miri interprets. A data race or a broken
//! acquire/release pairing in the gate shows up here as undefined behaviour
//! rather than as a count that merely looks odd, which is the failure class the
//! two process-global atomics actually risk.

#![forbid(unsafe_code)]

use std::hint::black_box;

use serial_test::serial;
use wireframe_bench_alloc::CountingAllocator;
use wireframe_bench_counting::{count_allocations, is_allocation_counting_enabled};

/// Process allocator for this test binary.
#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
#[serial]
fn installed_allocator_counts_a_positive_window() {
    let count = count_allocations(|| {
        let buffer: Vec<u8> = vec![0; 4096];
        black_box(&buffer);
        Ok(())
    })
    .expect("an infallible operation cannot fail");

    assert!(
        count > 0,
        "an installed counting allocator must observe the window's allocations; a zero count \
         means the `#[global_allocator]` static is not in effect"
    );
    assert!(
        !is_allocation_counting_enabled(),
        "the window must close once the operation returns"
    );
}

#[test]
#[serial]
fn installed_allocator_reports_zero_for_an_empty_window() {
    // The window brackets the closure alone, so a closure that does nothing must
    // observe no allocations. This is the control for the positive case above:
    // it fails if the counter is seeded or incremented outside a window.
    let count = count_allocations(|| Ok(())).expect("an infallible operation cannot fail");

    assert_eq!(count, 0, "an empty window must not observe allocations");
}

#[test]
#[serial]
fn installed_allocator_closes_the_window_after_a_failure() {
    let error = count_allocations(|| Err("deliberate failure".to_string()))
        .expect_err("a failing operation must surface its error");

    assert_eq!(error, "deliberate failure");
    assert!(
        !is_allocation_counting_enabled(),
        "a failed measurement must still close the window"
    );
}
