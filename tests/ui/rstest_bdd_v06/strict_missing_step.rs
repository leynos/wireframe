//! Strict validation rejects a feature step with no matching definition.

use rstest_bdd_macros::scenario;

/// Require strict validation to reject the otherwise valid scenario binding.
#[scenario(
    path = "rstest_bdd_v06/missing_step.feature",
    name = "Missing step is rejected"
)]
fn missing_step_is_rejected() {}

/// Supply the binary entry point required by trybuild.
fn main() {}
