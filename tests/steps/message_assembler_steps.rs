//! Step definitions for message assembler header parsing.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::message_assembler::{
    ContinuationHeaderSpec,
    FirstHeaderSpec,
    MessageAssemblerWorld,
    TestResult,
};

#[given(
    "a first frame header with key {key:u64} metadata length {metadata_len:usize} body length \
     {body_len:usize}"
)]
fn given_first_header(
    message_assembler_world: &mut MessageAssemblerWorld,
    key: u64,
    metadata_len: usize,
    body_len: usize,
) -> TestResult {
    message_assembler_world
        .set_first_header(FirstHeaderSpec::new(key, body_len).with_metadata_len(metadata_len))
}

#[given(
    "a first frame header with key {key:u64} body length {body_len:usize} total {total_len:usize}"
)]
fn given_first_header_with_total(
    message_assembler_world: &mut MessageAssemblerWorld,
    key: u64,
    body_len: usize,
    total_len: usize,
) -> TestResult {
    message_assembler_world.set_first_header(
        FirstHeaderSpec::new(key, body_len)
            .with_total_len(total_len)
            .with_last_flag(true),
    )
}

#[given(
    "a continuation header with key {key:u64} body length {body_len:usize} sequence {sequence:u32}"
)]
fn given_continuation_header_with_sequence(
    message_assembler_world: &mut MessageAssemblerWorld,
    key: u64,
    body_len: usize,
    sequence: u32,
) -> TestResult {
    message_assembler_world
        .set_continuation_header(ContinuationHeaderSpec::new(key, body_len).with_sequence(sequence))
}

#[given("a continuation header with key {key:u64} body length {body_len:usize}")]
fn given_continuation_header(
    message_assembler_world: &mut MessageAssemblerWorld,
    key: u64,
    body_len: usize,
) -> TestResult {
    message_assembler_world
        .set_continuation_header(ContinuationHeaderSpec::new(key, body_len).with_last_flag(true))
}

#[given("a wireframe app with a message assembler")]
fn given_app_with_message_assembler(
    message_assembler_world: &mut MessageAssemblerWorld,
) -> TestResult {
    message_assembler_world.set_app_with_message_assembler()
}

#[given("an invalid message header")]
fn given_invalid_header(message_assembler_world: &mut MessageAssemblerWorld) {
    message_assembler_world.set_invalid_payload();
}

#[when("the message assembler parses the header")]
fn when_parsing(message_assembler_world: &mut MessageAssemblerWorld) -> TestResult {
    message_assembler_world.parse_header()
}

#[then("the parsed header is {kind}")]
fn then_header_kind(
    message_assembler_world: &mut MessageAssemblerWorld,
    kind: String,
) -> TestResult {
    message_assembler_world.assert_header_kind(&kind)
}

#[then("the message key is {key:u64}")]
fn then_message_key(message_assembler_world: &mut MessageAssemblerWorld, key: u64) -> TestResult {
    message_assembler_world.assert_message_key(key)
}

#[then("the header metadata length is {metadata_len:usize}")]
fn then_metadata_len(
    message_assembler_world: &mut MessageAssemblerWorld,
    metadata_len: usize,
) -> TestResult {
    message_assembler_world.assert_metadata_len(metadata_len)
}

#[then("the body length is {body_len:usize}")]
fn then_body_len(
    message_assembler_world: &mut MessageAssemblerWorld,
    body_len: usize,
) -> TestResult {
    message_assembler_world.assert_body_len(body_len)
}

#[then("the header length is {header_len:usize}")]
fn then_header_len(
    message_assembler_world: &mut MessageAssemblerWorld,
    header_len: usize,
) -> TestResult {
    message_assembler_world.assert_header_len(header_len)
}

#[then("the total body length is absent")]
fn then_total_absent(message_assembler_world: &mut MessageAssemblerWorld) -> TestResult {
    message_assembler_world.assert_total_len(None)
}

#[then("the total body length is {total:usize}")]
fn then_total_present(
    message_assembler_world: &mut MessageAssemblerWorld,
    total: usize,
) -> TestResult {
    message_assembler_world.assert_total_len(Some(total))
}

#[then("the sequence is {sequence:u32}")]
fn then_sequence(message_assembler_world: &mut MessageAssemblerWorld, sequence: u32) -> TestResult {
    message_assembler_world.assert_sequence(Some(sequence))
}

#[then("the sequence is absent")]
fn then_sequence_absent(message_assembler_world: &mut MessageAssemblerWorld) -> TestResult {
    message_assembler_world.assert_sequence(None)
}

#[then("the frame is marked last {expected:bool}")]
fn then_is_last(message_assembler_world: &mut MessageAssemblerWorld, expected: bool) -> TestResult {
    message_assembler_world.assert_is_last(expected)
}

#[then("the parse fails with invalid data")]
fn then_invalid_data(message_assembler_world: &mut MessageAssemblerWorld) -> TestResult {
    message_assembler_world.assert_invalid_data_error()
}

#[then("the app exposes a message assembler")]
fn then_app_exposes_message_assembler(
    message_assembler_world: &mut MessageAssemblerWorld,
) -> TestResult {
    message_assembler_world.assert_message_assembler_configured()
}
