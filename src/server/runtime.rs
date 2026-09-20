//! Runtime control for [`WireframeServer`].

mod accept;
mod backoff;
mod server;
mod startup;
#[cfg(test)]
mod startup_tests;
mod supervisor;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) use accept::MockAcceptListener;
pub(super) use accept::{AcceptLoopOptions, PreambleHooks, accept_loop};
pub use backoff::BackoffConfig;
pub(in crate::server) use supervisor::SupervisorLifecycle;

#[cfg(test)]
pub(super) use super::WireframeServer;
