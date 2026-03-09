//! Step definitions for codec-error regression scenarios backed by
//! `wireframe_testing`.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::codec_error_regressions::{CodecErrorRegressionsWorld, TestResult};

#[given("a codec error regression world")]
fn given_codec_error_regression_world(
    codec_error_regressions_world: &mut CodecErrorRegressionsWorld,
) {
    codec_error_regressions_world.acquire();
}

#[when("an oversized framing error is recorded with its default policy")]
fn when_oversized_default_recorded(
    codec_error_regressions_world: &mut CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.record_default_oversized_error()
}

#[when("the default codec observes a clean EOF at frame boundary")]
fn when_clean_eof_observed(
    codec_error_regressions_world: &mut CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.observe_clean_close()
}

#[when("the default codec observes EOF during a partial header")]
fn when_partial_header_observed(
    codec_error_regressions_world: &mut CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.observe_partial_header()
}

#[when("the default codec observes EOF during a partial payload")]
fn when_partial_payload_observed(
    codec_error_regressions_world: &mut CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.observe_partial_payload()
}

#[when("a strict recovery hook records an oversized framing error")]
fn when_strict_hook_records_oversized(
    codec_error_regressions_world: &mut CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.record_strict_hook_override()
}

#[then("the observability handle reports {expected:u64} codec error for {error_type} and {policy}")]
fn then_codec_error_counter_matches(
    codec_error_regressions_world: &mut CodecErrorRegressionsWorld,
    expected: u64,
    error_type: String,
    policy: String,
) -> TestResult {
    codec_error_regressions_world.assert_codec_error_count(&error_type, &policy, expected)
}

#[then("the last EOF classification is clean close")]
fn then_last_eof_is_clean_close(
    codec_error_regressions_world: &CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.assert_last_clean_close()
}

#[then("the first partial EOF classification is mid header")]
fn then_first_partial_is_mid_header(
    codec_error_regressions_world: &CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.assert_first_partial_mid_header()
}

#[then("the second partial EOF classification is mid frame")]
fn then_second_partial_is_mid_frame(
    codec_error_regressions_world: &CodecErrorRegressionsWorld,
) -> TestResult {
    codec_error_regressions_world.assert_second_partial_mid_frame()
}
