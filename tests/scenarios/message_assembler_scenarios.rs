//! Scenario tests for message assembler header parsing.

use rstest_bdd_macros::scenario;

use crate::fixtures::message_assembler::*;

#[scenario(
    path = "tests/features/message_assembler.feature",
    name = "Builder exposes a configured message assembler"
)]
#[tokio::test(flavor = "current_thread")]
async fn message_assembler_builder_configured(message_assembler_world: MessageAssemblerWorld) {
    let _ = message_assembler_world;
}

#[scenario(
    path = "tests/features/message_assembler.feature",
    name = "Parsing a first frame header without total length"
)]
#[tokio::test(flavor = "current_thread")]
async fn message_assembler_first_header_without_total(
    message_assembler_world: MessageAssemblerWorld,
) {
    let _ = message_assembler_world;
}

#[scenario(
    path = "tests/features/message_assembler.feature",
    name = "Parsing a first frame header with total length"
)]
#[tokio::test(flavor = "current_thread")]
async fn message_assembler_first_header_with_total(message_assembler_world: MessageAssemblerWorld) {
    let _ = message_assembler_world;
}

#[scenario(
    path = "tests/features/message_assembler.feature",
    name = "Parsing a continuation header with sequence"
)]
#[tokio::test(flavor = "current_thread")]
async fn message_assembler_continuation_with_sequence(
    message_assembler_world: MessageAssemblerWorld,
) {
    let _ = message_assembler_world;
}

#[scenario(
    path = "tests/features/message_assembler.feature",
    name = "Parsing a continuation header without sequence"
)]
#[tokio::test(flavor = "current_thread")]
async fn message_assembler_continuation_without_sequence(
    message_assembler_world: MessageAssemblerWorld,
) {
    let _ = message_assembler_world;
}

#[scenario(
    path = "tests/features/message_assembler.feature",
    name = "Invalid header payload returns error"
)]
#[tokio::test(flavor = "current_thread")]
async fn message_assembler_invalid_payload(message_assembler_world: MessageAssemblerWorld) {
    let _ = message_assembler_world;
}
