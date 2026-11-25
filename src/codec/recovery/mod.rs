//! Recovery policy types and hooks for codec error handling.
//!
//! This module provides infrastructure for customizing how codec errors are
//! handled, including recovery policies, error context for structured logging,
//! and hooks for application-specific error handling.
//!
//! # Recovery Policies
//!
//! When a codec error occurs, the framework applies a recovery policy:
//!
//! - [`RecoveryPolicy::Drop`]: Discard the malformed frame and continue processing. Suitable for
//!   recoverable errors like oversized frames.
//! - [`RecoveryPolicy::Quarantine`]: Pause the connection temporarily before retrying. Useful for
//!   rate-limiting misbehaving clients.
//! - [`RecoveryPolicy::Disconnect`]: Terminate the connection immediately. Required for
//!   unrecoverable errors like I/O failures.
//!
//! # Custom Recovery Hooks
//!
//! Applications can customize error handling by implementing
//! [`RecoveryPolicyHook`]:
//!
//! ```
//! use std::time::Duration;
//!
//! use wireframe::codec::{CodecError, CodecErrorContext, RecoveryPolicy, RecoveryPolicyHook};
//!
//! struct StrictRecovery;
//!
//! impl RecoveryPolicyHook for StrictRecovery {
//!     fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
//!         // Disconnect on any error
//!         RecoveryPolicy::Disconnect
//!     }
//! }
//! ```
//!
//! # Runtime Behaviour
//!
//! Install a hook with
//! [`WireframeApp::with_recovery_hook`](crate::app::WireframeApp::with_recovery_hook)
//! and tune the shared limits with [`RecoveryConfig`] through
//! [`WireframeApp::recovery_config`](crate::app::WireframeApp::recovery_config).
//! The inbound connection loop then consults the installed hook for every
//! decoder failure on a live connection and applies the result:
//!
//! - [`RecoveryPolicy::Drop`] discards the malformed frame and reads the next one.
//! - [`RecoveryPolicy::Quarantine`] pauses the connection for the hook's `quarantine_duration`,
//!   purges expired assembly state, and then resumes.
//! - [`RecoveryPolicy::Disconnect`] ends the connection with the decoder's original error.
//!
//! The loop escalates a selected policy to [`RecoveryPolicy::Disconnect`] for
//! end-of-file failures, for a non-disconnecting policy that would discard a
//! frame the decoder did not consume, and for a [`RecoveryPolicy::Drop`] beyond
//! [`RecoveryConfig::max_consecutive_drops`]. It records
//! `wireframe_codec_errors_total` once per failure using the *applied* policy.
//! Without an installed hook every decoder failure disconnects.

mod config;
mod context;
mod hook;
mod policy;

pub use config::RecoveryConfig;
pub use context::CodecErrorContext;
pub use hook::{DefaultRecoveryPolicy, RecoveryPolicyHook};
pub use policy::RecoveryPolicy;

#[cfg(test)]
mod tests;
