//! Step definitions for aliased step-result regression scenarios.

use rstest_bdd_macros::{then, when};
use wireframe_testing::{TestError, TestResult};

use crate::fixtures::result_alias_dispatch::AliasOverrideValue;

/// Return a deliberate error through the step-result alias.
#[when("an aliased step returns an error")]
fn when_aliased_step_returns_error() -> TestResult {
    Err(TestError::Msg("aliased step failure".to_owned()))
}

/// Check the fixture's initial value before returning its replacement.
#[when("an aliased step returns the replacement value")]
fn when_aliased_step_returns_replacement(
    alias_override_value: &mut AliasOverrideValue,
) -> TestResult<AliasOverrideValue> {
    if alias_override_value.0 == 1 {
        Ok(AliasOverrideValue(2))
    } else {
        Err(TestError::Msg(format!(
            "expected initial fixture value 1, got {}",
            alias_override_value.0
        )))
    }
}

/// Check that the injected replacement matches the scenario expectation.
#[then("the replacement fixture is visible as {expected:u32}")]
fn then_replacement_fixture_is_visible(
    expected: u32,
    alias_override_value: &mut AliasOverrideValue,
) -> TestResult {
    if alias_override_value.0 == expected {
        Ok(())
    } else {
        Err(TestError::Msg(format!(
            "expected replacement value {expected}, got {}",
            alias_override_value.0
        )))
    }
}
