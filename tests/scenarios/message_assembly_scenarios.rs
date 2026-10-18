//! Scenario tests for message assembly multiplexing and continuity validation.

use rstest_bdd_macros::scenario;

use crate::fixtures::message_assembly::*;

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Single message assembly completes successfully"
)]
fn message_assembly_single_message(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Single-frame message completes immediately"
)]
fn message_assembly_single_frame(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Interleaved messages assemble independently"
)]
fn message_assembly_interleaved(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Out-of-order continuation is rejected but assembly retained"
)]
fn message_assembly_out_of_order(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Duplicate continuation is rejected but assembly retained"
)]
fn message_assembly_duplicate_continuation(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Continuation without first frame is rejected"
)]
fn message_assembly_missing_first(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Duplicate first frame is rejected"
)]
fn message_assembly_duplicate_first(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Message exceeding size limit is rejected"
)]
fn message_assembly_too_large(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Under-declared single-frame total is rejected"
)]
fn message_assembly_under_declared_single_frame(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Over-declared single-frame total is rejected"
)]
fn message_assembly_over_declared_single_frame(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Continuation exceeding the declared total is rejected"
)]
fn message_assembly_continuation_over_declared(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Final frame short of the declared total is rejected"
)]
fn message_assembly_final_frame_short(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Exactly declared multi-frame message completes"
)]
fn message_assembly_exact_declaration(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}

#[scenario(
    path = "tests/features/message_assembly.feature",
    name = "Expired assemblies are purged"
)]
fn message_assembly_expired(message_assembly_world: MessageAssemblyWorld) {
    drop(message_assembly_world);
}
