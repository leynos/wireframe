//! Cucumber test world implementations and shared helpers.
//!
//! Provides world types for behaviour-driven tests covering fragmentation,
//! correlation, panic recovery, stream termination, multi-packet channels, and
//! request parts. Shared utilities like `build_small_queues` keep individual
//! worlds focused on their respective scenarios.
#![cfg(not(loom))]

#[path = "../common/mod.rs"]
pub mod common;
pub use common::{TestResult, unused_listener};

#[path = "../common/terminator.rs"]
mod terminator;
pub(crate) use terminator::Terminator;

#[path = "../support.rs"]
mod support;

use wireframe::{app::Envelope, push::PushQueues, serializer::BincodeSerializer};

pub(crate) type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

pub(crate) fn build_small_queues<T: Send + 'static>()
-> Result<(PushQueues<T>, wireframe::push::PushHandle<T>), wireframe::push::PushConfigError> {
    support::builder::<T>().unlimited().build()
}

pub mod client_runtime;
pub mod correlation;
pub mod fragment;
pub mod multi_packet;
pub mod panic;
pub mod request_parts;
pub mod stream_end;
pub mod types;
