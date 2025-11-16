#![cfg(not(loom))]
//! Shared infrastructure for the Cucumber test worlds.
//!
//! This module re-exports the individual worlds and exposes utilities such as
//! `build_small_queues` so each world stays focused on its behaviour.

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
