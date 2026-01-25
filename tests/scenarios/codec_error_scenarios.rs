//! Scenario tests for codec error taxonomy and recovery behaviour.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_error::*;

#[scenario(
    path = "tests/features/codec_error.feature",
    name = "Clean EOF at frame boundary"
)]
#[tokio::test(flavor = "current_thread")]
async fn codec_error_clean_eof(codec_error_world: CodecErrorWorld) { let _ = codec_error_world; }

#[scenario(
    path = "tests/features/codec_error.feature",
    name = "Premature EOF mid-frame"
)]
#[tokio::test(flavor = "current_thread")]
async fn codec_error_mid_frame(codec_error_world: CodecErrorWorld) { let _ = codec_error_world; }

#[scenario(
    path = "tests/features/codec_error.feature",
    name = "Oversized frame produces framing error"
)]
#[tokio::test(flavor = "current_thread")]
async fn codec_error_oversized_frame(codec_error_world: CodecErrorWorld) {
    let _ = codec_error_world;
}

#[scenario(
    path = "tests/features/codec_error.feature",
    name = "Recovery policy defaults"
)]
#[tokio::test(flavor = "current_thread")]
async fn codec_error_recovery_defaults(codec_error_world: CodecErrorWorld) {
    let _ = codec_error_world;
}
