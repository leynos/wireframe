//! Step definitions for rstest-bdd tests.
//!
//! Step functions are synchronous and call async world methods via
//! `Runtime::new().block_on(...)`.

mod client_lifecycle_steps;
mod client_messaging_steps;
mod client_preamble_steps;
mod client_runtime_steps;
mod client_streaming_steps;
mod codec_error_steps;
mod codec_property_roundtrip_steps;
mod codec_stateful_steps;
mod correlation_steps;
mod fragment_steps;
mod memory_budgets_steps;
mod message_assembler_steps;
mod message_assembly_inbound_steps;
mod message_assembly_steps;
mod multi_packet_steps;
mod panic_steps;
mod request_parts_steps;
mod serializer_boundaries_steps;
mod stream_end_steps;
mod unified_codec_steps;

pub(crate) use message_assembly_steps::FrameId;
