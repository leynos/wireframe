//! Fixture definitions for rstest-bdd tests.
//!
//! Each world from the Cucumber tests is converted to an rstest fixture here.

pub mod client_lifecycle;
pub mod client_messaging;
pub mod client_preamble;
pub mod client_runtime;
pub mod codec_stateful;
pub mod correlation;
pub mod message_assembler;
pub mod message_assembly;
pub mod multi_packet;
pub mod panic;
pub mod request_parts;
pub mod stream_end;
