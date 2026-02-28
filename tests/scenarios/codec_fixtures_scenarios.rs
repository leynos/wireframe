//! Scenario tests for codec fixture behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_fixtures::*;

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Valid fixture decodes to expected payload"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn valid_fixture_decodes(codec_fixtures_world: CodecFixturesWorld) {}

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Oversized fixture is rejected by decoder"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn oversized_fixture_rejected(codec_fixtures_world: CodecFixturesWorld) {}

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Truncated fixture produces a decode error"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn truncated_fixture_error(codec_fixtures_world: CodecFixturesWorld) {}

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Correlated fixtures share the same transaction identifier"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn correlated_fixture_ids(codec_fixtures_world: CodecFixturesWorld) {}
