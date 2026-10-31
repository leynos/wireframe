//! Scenarios that pin aliased step-result dispatch behaviour.

use rstest_bdd_macros::scenario;

use crate::fixtures::result_alias_dispatch::{AliasOverrideValue, alias_override_value};

#[scenario(
    path = "tests/features/result_alias_dispatch.feature",
    name = "An aliased step error fails the scenario"
)]
#[should_panic(expected = "aliased step failure")]
fn aliased_step_error_is_propagated() {}

#[scenario(
    path = "tests/features/result_alias_dispatch.feature",
    name = "An aliased successful value overrides its fixture"
)]
fn aliased_success_overrides_fixture(alias_override_value: AliasOverrideValue) {
    let _ = alias_override_value;
}
