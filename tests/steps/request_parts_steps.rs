//! Steps for request parts behavioural tests.

use cucumber::{given, then, when};

use crate::world::{RequestPartsWorld, TestResult};

#[given(expr = "request parts with id {int} and correlation id {int}")]
fn given_parts_with_correlation(world: &mut RequestPartsWorld, id: u32, correlation_id: u64) {
    world.create_parts(id, Some(correlation_id), vec![]);
}

#[given(expr = "request parts with id {int} and no correlation id")]
fn given_parts_no_correlation(world: &mut RequestPartsWorld, id: u32) {
    world.create_parts(id, None, vec![]);
}

#[given(expr = "request parts with id {int}, no correlation id, and empty metadata")]
fn given_parts_empty_metadata(world: &mut RequestPartsWorld, id: u32) {
    world.create_parts(id, None, vec![]);
}

#[given(expr = "metadata bytes {int}, {int}, {int}")]
fn given_metadata_bytes_three(world: &mut RequestPartsWorld, b1: u8, b2: u8, b3: u8) -> TestResult {
    world.append_metadata_byte(b1)?;
    world.append_metadata_byte(b2)?;
    world.append_metadata_byte(b3)
}

#[given(expr = "metadata byte {int}")]
fn given_metadata_byte(world: &mut RequestPartsWorld, byte: u8) -> TestResult {
    world.append_metadata_byte(byte)
}

#[when(expr = "inheriting correlation id {int}")]
fn when_inherit_correlation(world: &mut RequestPartsWorld, correlation_id: u64) -> TestResult {
    world.inherit_correlation(Some(correlation_id))
}

#[when("inheriting no correlation id")]
fn when_inherit_no_correlation(world: &mut RequestPartsWorld) -> TestResult {
    world.inherit_correlation(None)
}

#[when(expr = "appending byte {int} to metadata")]
fn when_append_metadata(world: &mut RequestPartsWorld, byte: u8) -> TestResult {
    world.append_metadata_byte(byte)
}

#[then(expr = "the request id is {int}")]
fn then_id_is(world: &mut RequestPartsWorld, expected: u32) -> TestResult {
    world.assert_id(expected)
}

#[then(expr = "the correlation id is {int}")]
fn then_correlation_id_is(world: &mut RequestPartsWorld, expected: u64) -> TestResult {
    world.assert_correlation_id(Some(expected))
}

#[then(expr = "the metadata length is {int}")]
fn then_metadata_length_is(world: &mut RequestPartsWorld, expected: usize) -> TestResult {
    world.assert_metadata_length(expected)
}
