//! Recovery policy types and hooks for codec error handling.
//!
//! This module provides infrastructure for customising how codec errors are
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
//! Applications can customise error handling by implementing
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
