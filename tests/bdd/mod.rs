#![cfg(not(loom))]
//! rstest-bdd behavioural tests.
//!
//! This module contains the rstest-bdd-based BDD tests that are gradually
//! replacing the Cucumber test suite. These tests use the same `.feature`
//! files as the Cucumber tests but execute under the standard `cargo test`
//! harness with rstest fixtures.

mod fixtures;
mod scenarios;
mod steps;
