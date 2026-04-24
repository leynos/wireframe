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
