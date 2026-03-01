//! Fixture definitions for rstest-bdd tests.
//!
//! Each world from the former Cucumber tests is converted to an rstest fixture
//! here.

pub mod budget_enforcement;
pub mod client_lifecycle;
pub mod client_messaging;
pub mod client_preamble;
pub mod client_request_hooks;
pub mod client_runtime;
pub mod client_send_streaming;
pub mod client_streaming;
pub mod codec_error;
pub mod codec_performance_benchmarks;
pub mod codec_property_roundtrip;
pub mod codec_stateful;
pub mod codec_test_harness;
pub mod correlation;
pub mod derived_memory_budgets;
pub mod fragment;
pub mod interleaved_push_queues;
pub mod memory_budget_backpressure;
mod memory_budget_backpressure_tests;
pub mod memory_budget_hard_cap;
pub mod memory_budgets;
pub mod message_assembler;
pub mod message_assembly;
pub mod message_assembly_inbound;
pub mod multi_packet;
pub mod panic;
pub mod request_parts;
pub mod serializer_boundaries;
pub mod stream_end;
pub mod unified_codec;
