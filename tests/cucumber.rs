#![cfg(not(loom))]
//! Cucumber test runner for integration tests.
//!
//! Orchestrates thirteen distinct test suites:
//! - `PanicWorld`: Tests server resilience during connection panics
//! - `CorrelationWorld`: Tests correlation ID propagation in multi-frame responses
//! - `StreamEndWorld`: Verifies end-of-stream signalling
//! - `MultiPacketWorld`: Tests channel-backed multi-packet response delivery
//! - `FragmentWorld`: Tests fragment metadata enforcement and reassembly primitives
//! - `MessageAssemblerWorld`: Tests message assembler header parsing
//! - `MessageAssemblyWorld`: Tests message assembly multiplexing and continuity
//! - `ClientRuntimeWorld`: Tests client runtime configuration and framing behaviour
//! - `ClientMessagingWorld`: Tests client messaging APIs with correlation ID support
//! - `CodecStatefulWorld`: Tests instance-aware codec sequence counters
//! - `RequestPartsWorld`: Tests request parts metadata handling
//! - `ClientPreambleWorld`: Tests client preamble exchange and callbacks
//! - `ClientLifecycleWorld`: Tests client connection lifecycle hooks
//! - `CodecErrorWorld`: Tests codec error taxonomy and recovery policies
//!
//! # Example
//!
//! The runner executes feature files sequentially:
//! ```text
//! tests/features/connection_panic.feature    -> PanicWorld context
//! tests/features/correlation_id.feature      -> CorrelationWorld context
//! tests/features/stream_end.feature          -> StreamEndWorld context
//! tests/features/multi_packet.feature        -> MultiPacketWorld context
//! tests/features/fragment.feature            -> FragmentWorld context
//! tests/features/message_assembler.feature   -> MessageAssemblerWorld context
//! tests/features/message_assembly.feature    -> MessageAssemblyWorld context
//! tests/features/client_runtime.feature      -> ClientRuntimeWorld context
//! tests/features/client_messaging.feature    -> ClientMessagingWorld context
//! tests/features/codec_stateful.feature      -> CodecStatefulWorld context
//! tests/features/request_parts.feature       -> RequestPartsWorld context
//! tests/features/client_preamble.feature     -> ClientPreambleWorld context
//! tests/features/client_lifecycle.feature    -> ClientLifecycleWorld context
//! tests/features/codec_error.feature         -> CodecErrorWorld context
//! ```
//!
//! Each context provides specialised step definitions and state management
//! for their respective test scenarios.

mod steps;
mod world;

use cucumber::World;
use world::{
    ClientLifecycleWorld,
    ClientMessagingWorld,
    ClientPreambleWorld,
    ClientRuntimeWorld,
    CodecErrorWorld,
    CodecStatefulWorld,
    CorrelationWorld,
    FragmentWorld,
    MessageAssemblerWorld,
    MessageAssemblyWorld,
    MultiPacketWorld,
    PanicWorld,
    RequestPartsWorld,
    StreamEndWorld,
};

#[tokio::main]
async fn main() {
    PanicWorld::run("tests/features/connection_panic.feature").await;
    CorrelationWorld::run("tests/features/correlation_id.feature").await;
    StreamEndWorld::run("tests/features/stream_end.feature").await;
    MultiPacketWorld::run("tests/features/multi_packet.feature").await;
    FragmentWorld::run("tests/features/fragment.feature").await;
    MessageAssemblerWorld::run("tests/features/message_assembler.feature").await;
    MessageAssemblyWorld::run("tests/features/message_assembly.feature").await;
    ClientRuntimeWorld::run("tests/features/client_runtime.feature").await;
    ClientMessagingWorld::run("tests/features/client_messaging.feature").await;
    CodecStatefulWorld::run("tests/features/codec_stateful.feature").await;
    RequestPartsWorld::run("tests/features/request_parts.feature").await;
    ClientPreambleWorld::run("tests/features/client_preamble.feature").await;
    ClientLifecycleWorld::run("tests/features/client_lifecycle.feature").await;
    CodecErrorWorld::run("tests/features/codec_error.feature").await;
}
