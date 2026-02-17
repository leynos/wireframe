//! Fixture definitions for rstest-bdd tests.
//!
//! Each world from the former Cucumber tests is converted to an rstest fixture
//! here.

pub mod client_lifecycle;
pub mod client_messaging;
pub mod client_preamble;
pub mod client_runtime;
pub mod client_streaming;
pub mod codec_error;
pub mod codec_stateful;
pub mod correlation;
pub mod fragment;
pub mod memory_budgets;
pub mod message_assembler;
pub mod message_assembly;
pub mod message_assembly_inbound;
pub mod multi_packet;
pub mod panic;
pub mod request_parts;
pub mod stream_end;
pub mod unified_codec;
