//! Integration checks for the shared verification harness.

use std::num::NonZeroUsize;

use rstest::rstest;
use wireframe_verification::{
    connection_model::PlaceholderConnectionModel,
    harness::{VerificationBounds, assert_model_properties_with_bounds},
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
