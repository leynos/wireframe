//! Step definitions for inbound message assembly integration scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::message_assembly_inbound::{MessageAssemblyInboundWorld, TestResult};

#[given("an inbound app with message assembly timeout {timeout_ms:u64} milliseconds")]
fn given_inbound_app(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
    timeout_ms: u64,
) -> TestResult {
    message_assembly_inbound_world.start_app(timeout_ms)
}

#[when("a first frame for key {key:u64} with body {body:string} arrives")]
fn when_first_frame_arrives(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
    key: u64,
    body: String,
) -> TestResult {
    message_assembly_inbound_world.send_first_frame(key, &body)
}

#[when(
    "a continuation frame for key {key:u64} sequence {sequence:u32} with body {body:string} \
     arrives"
)]
fn when_continuation_frame_arrives(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    message_assembly_inbound_world.send_continuation_frame(key, sequence, &body)
}

#[when(
    "a final continuation frame for key {key:u64} sequence {sequence:u32} with body {body:string} \
     arrives"
)]
fn when_final_continuation_frame_arrives(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    message_assembly_inbound_world.send_final_continuation_frame(key, sequence, &body)
}

#[when("time advances by {millis:u64} milliseconds")]
fn when_time_advances(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
    millis: u64,
) -> TestResult {
    message_assembly_inbound_world.wait_millis(millis)
}

#[then("the handler eventually receives payload {payload:string}")]
fn then_handler_receives_payload(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
    payload: String,
) -> TestResult {
    message_assembly_inbound_world.assert_received_payload(&payload)
}

#[then("the handler receives {count:usize} payloads")]
fn then_handler_receives_count(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
    count: usize,
) -> TestResult {
    message_assembly_inbound_world.assert_received_count(count)
}

#[then("no send error is recorded")]
fn then_no_send_error(
    message_assembly_inbound_world: &mut MessageAssemblyInboundWorld,
) -> TestResult {
    message_assembly_inbound_world.assert_no_send_error()
}
