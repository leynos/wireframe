//! Scenario tests for inbound message assembly integration.

use rstest_bdd_macros::scenario;

use crate::fixtures::message_assembly_inbound::*;

#[scenario(
    path = "tests/features/message_assembly_inbound.feature",
    name = "Interleaved assembly dispatches completed payloads"
)]
fn message_assembly_inbound_interleaved(
    message_assembly_inbound_world: MessageAssemblyInboundWorld,
) {
    drop(message_assembly_inbound_world);
}

#[scenario(
    path = "tests/features/message_assembly_inbound.feature",
    name = "Ordering violations are rejected and recovery can complete"
)]
fn message_assembly_inbound_ordering_violation(
    message_assembly_inbound_world: MessageAssemblyInboundWorld,
) {
    drop(message_assembly_inbound_world);
}

#[scenario(
    path = "tests/features/message_assembly_inbound.feature",
    name = "Timeout purges partial assembly before continuation arrives"
)]
fn message_assembly_inbound_timeout(message_assembly_inbound_world: MessageAssemblyInboundWorld) {
    drop(message_assembly_inbound_world);
}
