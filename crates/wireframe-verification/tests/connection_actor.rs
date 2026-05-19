//! Integration checks for the placeholder connection-actor model.

use rstest::rstest;
use wireframe_verification::{
    connection_model::PlaceholderConnectionModel,
    harness::assert_model_properties,
};

#[rstest]
fn placeholder_connection_model_satisfies_repository_properties() {
    assert_model_properties(PlaceholderConnectionModel::default());
}

#[rstest]
#[case(3)]
#[case(6)]
#[case(10)]
fn placeholder_model_satisfies_properties_under_varied_bounds(#[case] max_steps: u8) {
    assert_model_properties(PlaceholderConnectionModel::new(max_steps));
}
