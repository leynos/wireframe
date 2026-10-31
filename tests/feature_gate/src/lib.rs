//! Downstream feature-gate fixtures for the client pool's `test-support` API.
//!
//! The package exists to answer one question the workspace's own tests cannot:
//! do `ClientPool::pool_size` and `PooledClientLease::slot_id` disappear when
//! `test-support` is off? Trybuild forwards the harness features to its fixture
//! and the root manifest's `wireframe_testing` development dependency enables
//! `test-support`, so no trybuild case can observe the absent state. This
//! package depends on `wireframe` alone; see its `Cargo.toml` for the full
//! rationale.

#![forbid(unsafe_code)]

/// Compiles against the stable public pool API in every configuration.
pub mod public_api;
/// Compiles only when the `test-support` feature is enabled.
#[cfg(feature = "test-support")]
pub mod support_accessors;
