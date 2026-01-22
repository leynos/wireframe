//! Step definitions for `request_parts` behavioural tests.
//!
//! All steps are synchronous. No async operations are needed for this world.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::request_parts::{RequestPartsWorld, TestResult};

#[given("request parts with id {id:u32} and correlation id {cid:u64}")]
fn given_parts_with_correlation(request_parts_world: &mut RequestPartsWorld, id: u32, cid: u64) {
    request_parts_world.create_parts(id, Some(cid), vec![]);
}

#[given("request parts with id {id:u32} and no correlation id")]
fn given_parts_no_correlation(request_parts_world: &mut RequestPartsWorld, id: u32) {
    request_parts_world.create_parts(id, None, vec![]);
}

// Deliberately duplicates `given_parts_no_correlation` to provide distinct
// Gherkin phrasing: scenarios that later add metadata use the shorter form,
// while this form explicitly states the empty-metadata precondition.
#[given("request parts with id {id:u32}, no correlation id, and empty metadata")]
fn given_parts_empty_metadata(request_parts_world: &mut RequestPartsWorld, id: u32) {
    request_parts_world.create_parts(id, None, vec![]);
}

#[given("metadata bytes {b1:u8}, {b2:u8}, {b3:u8}")]
fn given_metadata_bytes_three(
    request_parts_world: &mut RequestPartsWorld,
    b1: u8,
    b2: u8,
    b3: u8,
) -> TestResult {
    request_parts_world.append_metadata_byte(b1)?;
    request_parts_world.append_metadata_byte(b2)?;
    request_parts_world.append_metadata_byte(b3)
}

#[given("metadata byte {byte:u8}")]
fn given_metadata_byte(request_parts_world: &mut RequestPartsWorld, byte: u8) -> TestResult {
    request_parts_world.append_metadata_byte(byte)
}

#[when("inheriting correlation id {cid:u64}")]
fn when_inherit_correlation(request_parts_world: &mut RequestPartsWorld, cid: u64) -> TestResult {
    request_parts_world.inherit_correlation(Some(cid))
}

#[when("inheriting no correlation id")]
fn when_inherit_no_correlation(request_parts_world: &mut RequestPartsWorld) -> TestResult {
    request_parts_world.inherit_correlation(None)
}

#[when("appending byte {byte:u8} to metadata")]
fn when_append_metadata(request_parts_world: &mut RequestPartsWorld, byte: u8) -> TestResult {
    request_parts_world.append_metadata_byte(byte)
}

#[then("the request id is {expected:u32}")]
fn then_id_is(request_parts_world: &mut RequestPartsWorld, expected: u32) -> TestResult {
    request_parts_world.assert_id(expected)
}

#[then("the correlation id is {expected:u64}")]
fn then_correlation_id_is(
    request_parts_world: &mut RequestPartsWorld,
    expected: u64,
) -> TestResult {
    request_parts_world.assert_correlation_id(Some(expected))
}

#[then("the correlation id is absent")]
fn then_correlation_id_is_absent(request_parts_world: &mut RequestPartsWorld) -> TestResult {
    request_parts_world.assert_correlation_id(None)
}

#[then("the metadata length is {expected:usize}")]
fn then_metadata_length_is(
    request_parts_world: &mut RequestPartsWorld,
    expected: usize,
) -> TestResult {
    request_parts_world.assert_metadata_length(expected)
}
