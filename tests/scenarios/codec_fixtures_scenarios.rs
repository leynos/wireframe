//! Scenario tests for codec fixture behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_fixtures::*;

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Valid fixture decodes to expected payload"
)]
fn valid_fixture_decodes(codec_fixtures_world: CodecFixturesWorld) { let _ = codec_fixtures_world; }

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Oversized fixture is rejected by decoder"
)]
fn oversized_fixture_rejected(codec_fixtures_world: CodecFixturesWorld) {
    let _ = codec_fixtures_world;
}

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Truncated fixture produces a decode error"
)]
fn truncated_fixture_error(codec_fixtures_world: CodecFixturesWorld) {
    let _ = codec_fixtures_world;
}

#[scenario(
    path = "tests/features/codec_fixtures.feature",
    name = "Correlated fixtures share the same transaction identifier"
)]
fn correlated_fixture_ids(codec_fixtures_world: CodecFixturesWorld) {
    let _ = codec_fixtures_world;
}
