//! Cucumber test world implementations and shared helpers.
#![cfg(not(loom))]
//!
//! Provides world types for behaviour-driven tests covering fragmentation,
//! correlation, panic recovery, stream termination, and multi-packet channels.
//! Shared utilities like `build_small_queues` keep individual worlds focused on
//! their respective scenarios.

#[path = "../common/mod.rs"]
mod common;
pub(crate) use common::unused_listener;

#[path = "../common/terminator.rs"]
mod terminator;
pub(crate) use terminator::Terminator;

#[path = "../support.rs"]
mod support;

use wireframe::{app::Envelope, push::PushQueues, serializer::BincodeSerializer};

pub(crate) type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

pub(crate) fn build_small_queues<T: Send + 'static>()
-> (PushQueues<T>, wireframe::push::PushHandle<T>) {
    support::builder::<T>()
        .unlimited()
        .build()
        .expect("failed to build PushQueues")
}

pub mod correlation;
pub mod fragment;
pub mod multi_packet;
pub mod panic;
pub mod stream_end;
