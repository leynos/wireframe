//! Integration checks for the shared verification harness.

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
            max_depth: 8,
            max_state_count: 5_000,
        },
    );
}
