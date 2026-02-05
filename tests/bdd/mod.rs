//! rstest-bdd behavioural tests.
//!
//! This module contains the rstest-bdd-based BDD tests that replaced the
//! former Cucumber test suite. These tests use the same `.feature` files but
//! execute under the standard `cargo test` harness with rstest fixtures.

#![cfg(not(loom))]

#[path = "../common/terminator.rs"]
mod terminator;

#[path = "../support.rs"]
mod support;

use wireframe::{app::Envelope, push::PushQueues, serializer::BincodeSerializer};

pub(crate) type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

pub(crate) fn build_small_queues<T: Send + 'static>()
-> Result<(PushQueues<T>, wireframe::push::PushHandle<T>), wireframe::push::PushConfigError> {
    support::builder::<T>().unlimited().build()
}

#[path = "../fixtures/mod.rs"]
mod fixtures;

#[path = "../scenarios/mod.rs"]
mod scenarios;
