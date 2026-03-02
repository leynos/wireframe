//! Scenario tests for observability harness behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::test_observability::*;

#[scenario(
    path = "tests/features/test_observability.feature",
    name = "Metrics are captured via the observability handle"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn metrics_captured(test_observability_world: TestObservabilityWorld) {}

#[scenario(
    path = "tests/features/test_observability.feature",
    name = "Logs are captured via the observability handle"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn logs_captured(test_observability_world: TestObservabilityWorld) {}

#[scenario(
    path = "tests/features/test_observability.feature",
    name = "Clear resets captured state"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn clear_resets_state(test_observability_world: TestObservabilityWorld) {}

#[scenario(
    path = "tests/features/test_observability.feature",
    name = "Absent metrics return zero"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn absent_metrics_zero(test_observability_world: TestObservabilityWorld) {}
