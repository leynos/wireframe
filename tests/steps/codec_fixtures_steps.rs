//! Step definitions for codec fixture behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::codec_fixtures::{CodecFixturesWorld, TestResult};

#[given("a Hotline codec allowing fixtures up to {max_frame_length:usize} bytes")]
fn given_codec(
    codec_fixtures_world: &mut CodecFixturesWorld,
    max_frame_length: usize,
) -> TestResult {
    codec_fixtures_world.configure_codec(max_frame_length)
}

#[when("a valid fixture frame is decoded")]
fn when_valid_decoded(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.decode_valid_fixture()
}

#[when("an oversized fixture frame is decoded")]
fn when_oversized_decoded(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.decode_oversized_fixture()
}

#[when("a truncated fixture frame is decoded")]
fn when_truncated_decoded(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.decode_truncated_fixture()
}

#[when("correlated fixture frames are decoded")]
fn when_correlated_decoded(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.decode_correlated_fixture()
}

#[then("the decoded payload matches the fixture input")]
fn then_payload_matches(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.verify_payload_matches()
}

#[then("the decoder reports an invalid data error")]
fn then_invalid_data(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.verify_invalid_data_error()
}

#[then("the decoder reports bytes remaining on stream")]
fn then_bytes_remaining(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.verify_bytes_remaining_error()
}

#[then("all frames have the expected transaction identifier")]
fn then_correlated_ids(codec_fixtures_world: &mut CodecFixturesWorld) -> TestResult {
    codec_fixtures_world.verify_correlated_transaction_ids()
}
