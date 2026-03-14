//! Scenario tests for codec-error regressions backed by `wireframe_testing`.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_error_regressions::*;

#[scenario(
    path = "tests/features/codec_error_regressions.feature",
    name = "Oversized payload is classified as framing drop and counted"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn oversized_default_policy(codec_error_regressions_world: CodecErrorRegressionsWorld) {}

#[scenario(
    path = "tests/features/codec_error_regressions.feature",
    name = "Clean EOF at frame boundary maps to eof disconnect"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn clean_close_mapping(codec_error_regressions_world: CodecErrorRegressionsWorld) {}

#[scenario(
    path = "tests/features/codec_error_regressions.feature",
    name = "Partial header and partial payload closures are distinguished"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn partial_eof_distinction(codec_error_regressions_world: CodecErrorRegressionsWorld) {}

#[scenario(
    path = "tests/features/codec_error_regressions.feature",
    name = "A custom recovery hook overrides the recorded policy"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn custom_hook_override(codec_error_regressions_world: CodecErrorRegressionsWorld) {}
