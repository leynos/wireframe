//! Steps for request parts behavioural tests.

use cucumber::{given, then, when};

use crate::world::{
    RequestPartsWorld,
    TestResult,
    types::{CorrelationId, MetadataByte, MetadataLength, RequestId},
};

#[given(expr = "request parts with id {word} and correlation id {word}")]
fn given_parts_with_correlation(world: &mut RequestPartsWorld, id: RequestId, cid: CorrelationId) {
    world.create_parts(id.0, Some(cid.0), vec![]);
}

#[given(expr = "request parts with id {word} and no correlation id")]
fn given_parts_no_correlation(world: &mut RequestPartsWorld, id: RequestId) {
    world.create_parts(id.0, None, vec![]);
}

#[given(expr = "request parts with id {word}, no correlation id, and empty metadata")]
fn given_parts_empty_metadata(world: &mut RequestPartsWorld, id: RequestId) {
    world.create_parts(id.0, None, vec![]);
}

#[given(expr = "metadata bytes {word}, {word}, {word}")]
fn given_metadata_bytes_three(
    world: &mut RequestPartsWorld,
    b1: MetadataByte,
    b2: MetadataByte,
    b3: MetadataByte,
) -> TestResult {
    world.append_metadata_byte(b1.0)?;
    world.append_metadata_byte(b2.0)?;
    world.append_metadata_byte(b3.0)
}

#[given(expr = "metadata byte {word}")]
fn given_metadata_byte(world: &mut RequestPartsWorld, byte: MetadataByte) -> TestResult {
    world.append_metadata_byte(byte.0)
}

#[when(expr = "inheriting correlation id {word}")]
fn when_inherit_correlation(world: &mut RequestPartsWorld, cid: CorrelationId) -> TestResult {
    world.inherit_correlation(Some(cid.0))
}

#[when("inheriting no correlation id")]
fn when_inherit_no_correlation(world: &mut RequestPartsWorld) -> TestResult {
    world.inherit_correlation(None)
}

#[when(expr = "appending byte {word} to metadata")]
fn when_append_metadata(world: &mut RequestPartsWorld, byte: MetadataByte) -> TestResult {
    world.append_metadata_byte(byte.0)
}

#[then(expr = "the request id is {word}")]
fn then_id_is(world: &mut RequestPartsWorld, expected: RequestId) -> TestResult {
    world.assert_id(expected.0)
}

#[then(expr = "the correlation id is {word}")]
fn then_correlation_id_is(world: &mut RequestPartsWorld, expected: CorrelationId) -> TestResult {
    world.assert_correlation_id(Some(expected.0))
}

#[then(expr = "the metadata length is {word}")]
fn then_metadata_length_is(world: &mut RequestPartsWorld, expected: MetadataLength) -> TestResult {
    world.assert_metadata_length(expected.0)
}
