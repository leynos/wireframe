//! Scenario test functions for rstest-bdd.
//!
//! Each scenario from the `.feature` files has a corresponding `#[scenario]`
//! test function here.

// Load step definitions first so compile-time validation can see them.
#[path = "../steps/mod.rs"]
pub(crate) mod steps;

mod budget_enforcement_scenarios;
mod client_lifecycle_scenarios;
mod client_messaging_scenarios;
mod client_preamble_scenarios;
mod client_runtime_scenarios;
mod client_streaming_scenarios;
mod codec_error_scenarios;
mod codec_property_roundtrip_scenarios;
mod codec_stateful_scenarios;
mod correlation_scenarios;
mod fragment_scenarios;
mod interleaved_push_queues_scenarios;
mod memory_budgets_scenarios;
mod message_assembler_scenarios;
mod message_assembly_inbound_scenarios;
mod message_assembly_scenarios;
mod multi_packet_scenarios;
mod panic_scenarios;
mod request_parts_scenarios;
mod stream_end_scenarios;
mod unified_codec_scenarios;
