//! Fixtures for aliased step-result regression scenarios.

use rstest::fixture;

/// Value used to verify that a successful aliased step result replaces a fixture.
#[derive(Debug, PartialEq, Eq)]
pub struct AliasOverrideValue(pub u32);

/// Provide the initial value that a step result must replace.
#[rustfmt::skip]
#[fixture]
pub fn alias_override_value() -> AliasOverrideValue {
    AliasOverrideValue(1)
}
