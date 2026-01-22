#![cfg(not(loom))]
//! rstest-bdd behavioural tests.
//!
//! This module contains the rstest-bdd-based BDD tests that are gradually
//! replacing the Cucumber test suite. These tests use the same `.feature`
//! files as the Cucumber tests but execute under the standard `cargo test`
//! harness with rstest fixtures.

// Re-export common utilities from the parent tests directory
#[path = "../common/mod.rs"]
pub mod common;

#[path = "../support.rs"]
mod support;

use wireframe::{app::Envelope, push::PushQueues, serializer::BincodeSerializer};

#[expect(dead_code, reason = "shared type not used by all test scenarios yet")]
pub(crate) type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

pub(crate) fn build_small_queues<T: Send + 'static>()
-> Result<(PushQueues<T>, wireframe::push::PushHandle<T>), wireframe::push::PushConfigError> {
    support::builder::<T>().unlimited().build()
}

mod fixtures;
mod scenarios;
mod steps;
