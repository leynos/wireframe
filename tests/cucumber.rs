#![cfg(not(loom))]
//! Cucumber test runner for integration tests.
//!
//! Orchestrates seven distinct test suites:
//! - `PanicWorld`: Tests server resilience during connection panics
//! - `CorrelationWorld`: Tests correlation ID propagation in multi-frame responses
//! - `StreamEndWorld`: Verifies end-of-stream signalling
//! - `MultiPacketWorld`: Tests channel-backed multi-packet response delivery
//! - `FragmentWorld`: Tests fragment metadata enforcement and reassembly primitives
//! - `ClientRuntimeWorld`: Tests client runtime configuration and framing behaviour
//! - `RequestPartsWorld`: Tests request parts metadata handling
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
//! tests/features/client_runtime.feature      -> ClientRuntimeWorld context
//! tests/features/request_parts.feature       -> RequestPartsWorld context
//! ```
//!
//! Each context provides specialised step definitions and state management
//! for their respective test scenarios.

mod steps;
mod world;

use cucumber::World;
use world::{
    ClientRuntimeWorld,
    CorrelationWorld,
    FragmentWorld,
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
    ClientRuntimeWorld::run("tests/features/client_runtime.feature").await;
    RequestPartsWorld::run("tests/features/request_parts.feature").await;
}
