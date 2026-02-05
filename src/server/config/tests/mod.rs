//! Tests for server configuration utilities.
//!
//! This module exercises the `WireframeServer` builder, covering worker counts,
//! binding behaviour, preamble handling, handler registration, and method
//! chaining. Fixtures from `test_util` provide shared setup and parameterised
//! cases via `rstest`.

mod tests_backoff;
mod tests_basic;
mod tests_binding;
mod tests_integration;
mod tests_preamble;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreambleHandlerKind {
    Success,
    Failure,
}

fn expected_default_worker_count() -> usize {
    // Mirror the default worker logic to keep tests aligned with `WireframeServer::new`.
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}
