//! Integration checks for the shared verification harness.

use std::num::NonZeroUsize;

use rstest::rstest;
use wireframe_verification::{
    connection_model::PlaceholderConnectionModel,
    harness::{
        DEFAULT_TARGET_MAX_DEPTH,
        DEFAULT_TARGET_STATE_COUNT,
        VerificationBounds,
        assert_model_properties_with_bounds,
    },
};

#[rstest]
fn shared_harness_runs_placeholder_model_with_explicit_bounds() {
    assert_model_properties_with_bounds(
        PlaceholderConnectionModel::new(6),
        VerificationBounds {
            max_depth: NonZeroUsize::new(8).expect("8 is non-zero"),
            max_state_count: NonZeroUsize::new(5_000).expect("5_000 is non-zero"),
        },
    );
}

#[rstest]
fn default_verification_bounds_match_exported_constants() {
    let bounds = VerificationBounds::default();

    assert_eq!(bounds.max_depth, DEFAULT_TARGET_MAX_DEPTH);
    assert_eq!(bounds.max_state_count, DEFAULT_TARGET_STATE_COUNT);
}
