//! Step definitions for message assembler header parsing.

use cucumber::{given, then, when};

use crate::world::{ContinuationHeaderSpec, FirstHeaderSpec, MessageAssemblerWorld};

#[given(expr = "a first frame header with key {int} metadata length {int} body length {int}")]
fn given_first_header(
    world: &mut MessageAssemblerWorld,
    key: u64,
    metadata_len: usize,
    body_len: usize,
) -> crate::world::TestResult {
    world.set_first_header(FirstHeaderSpec {
        key,
        metadata_len,
        body_len,
        total_len: None,
        is_last: false,
    })
}

#[given(expr = "a first frame header with key {int} body length {int} total {int}")]
fn given_first_header_with_total(
    world: &mut MessageAssemblerWorld,
    key: u64,
    body_len: usize,
    total_len: usize,
) -> crate::world::TestResult {
    world.set_first_header(FirstHeaderSpec {
        key,
        metadata_len: 0,
        body_len,
        total_len: Some(total_len),
        is_last: true,
    })
}

#[given(expr = "a continuation header with key {int} body length {int} sequence {int}")]
fn given_continuation_header_with_sequence(
    world: &mut MessageAssemblerWorld,
    key: u64,
    body_len: usize,
    sequence: u32,
) -> crate::world::TestResult {
    world.set_continuation_header(ContinuationHeaderSpec {
        key,
        body_len,
        sequence: Some(sequence),
        is_last: false,
    })
}

#[given(expr = "a continuation header with key {int} body length {int}")]
fn given_continuation_header(
    world: &mut MessageAssemblerWorld,
    key: u64,
    body_len: usize,
) -> crate::world::TestResult {
    world.set_continuation_header(ContinuationHeaderSpec {
        key,
        body_len,
        sequence: None,
        is_last: true,
    })
}

#[given("an invalid message header")]
fn given_invalid_header(world: &mut MessageAssemblerWorld) { world.set_invalid_payload(); }

#[when("the message assembler parses the header")]
fn when_parsing(world: &mut MessageAssemblerWorld) -> crate::world::TestResult {
    world.parse_header()
}

#[then(expr = "the parsed header is {word}")]
fn then_header_kind(world: &mut MessageAssemblerWorld, kind: String) -> crate::world::TestResult {
    let result = world.assert_header_kind(&kind);
    drop(kind);
    result
}

#[then(expr = "the message key is {int}")]
fn then_message_key(world: &mut MessageAssemblerWorld, key: u64) -> crate::world::TestResult {
    world.assert_message_key(key)
}

#[then(expr = "the metadata length is {int}")]
fn then_metadata_len(
    world: &mut MessageAssemblerWorld,
    metadata_len: usize,
) -> crate::world::TestResult {
    world.assert_metadata_len(metadata_len)
}

#[then(expr = "the body length is {int}")]
fn then_body_len(world: &mut MessageAssemblerWorld, body_len: usize) -> crate::world::TestResult {
    world.assert_body_len(body_len)
}

#[then("the total body length is absent")]
fn then_total_absent(world: &mut MessageAssemblerWorld) -> crate::world::TestResult {
    world.assert_total_len(None)
}

#[then(expr = "the total body length is {int}")]
fn then_total_present(world: &mut MessageAssemblerWorld, total: usize) -> crate::world::TestResult {
    world.assert_total_len(Some(total))
}

#[then(expr = "the sequence is {int}")]
fn then_sequence(world: &mut MessageAssemblerWorld, sequence: u32) -> crate::world::TestResult {
    world.assert_sequence(Some(sequence))
}

#[then("the sequence is absent")]
fn then_sequence_absent(world: &mut MessageAssemblerWorld) -> crate::world::TestResult {
    world.assert_sequence(None)
}

#[then(expr = "the frame is marked last {word}")]
fn then_is_last(world: &mut MessageAssemblerWorld, expected: String) -> crate::world::TestResult {
    let expected_str = expected;
    let expected = expected_str == "true";
    drop(expected_str);
    world.assert_is_last(expected)
}

#[then("the parse fails with invalid data")]
fn then_invalid_data(world: &mut MessageAssemblerWorld) -> crate::world::TestResult {
    world.assert_invalid_data_error()
}
