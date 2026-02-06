//! Hooks for customising codec error recovery behaviour.

use std::time::Duration;

use super::{CodecErrorContext, RecoveryPolicy};
use crate::codec::error::CodecError;

/// Hook trait for customising codec error recovery behaviour.
///
/// Implementations can override default recovery policies based on
/// application-specific requirements or connection state.
///
/// # Default Implementation
///
/// The default implementation ([`DefaultRecoveryPolicy`]) delegates to
/// [`CodecError::default_recovery_policy`] for all errors.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use wireframe::codec::{
///     CodecError,
///     CodecErrorContext,
///     EofError,
///     RecoveryPolicy,
///     RecoveryPolicyHook,
/// };
///
/// /// Quarantine connections that close unexpectedly.
/// struct QuarantineOnPrematureEof;
///
/// impl RecoveryPolicyHook for QuarantineOnPrematureEof {
///     fn recovery_policy(&self, error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
///         match error {
///             CodecError::Eof(EofError::MidFrame { .. }) => RecoveryPolicy::Quarantine,
///             _ => error.default_recovery_policy(),
///         }
///     }
///
///     fn quarantine_duration(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> Duration {
///         Duration::from_secs(60)
///     }
/// }
/// ```
pub trait RecoveryPolicyHook: Send + Sync {
    /// Determine the recovery policy for a codec error.
    ///
    /// The default implementation delegates to
    /// [`CodecError::default_recovery_policy`].
    fn recovery_policy(&self, error: &CodecError, ctx: &CodecErrorContext) -> RecoveryPolicy {
        let _ = ctx;
        error.default_recovery_policy()
    }

    /// Returns the quarantine duration when [`RecoveryPolicy::Quarantine`] is
    /// selected.
    ///
    /// Default: 30 seconds.
    fn quarantine_duration(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> Duration {
        Duration::from_secs(30)
    }

    /// Called before applying the recovery policy.
    ///
    /// Use this hook for logging, metrics emission, or state updates. The
    /// default implementation does nothing.
    fn on_error(&self, _error: &CodecError, _ctx: &CodecErrorContext, _policy: RecoveryPolicy) {}
}

/// Default recovery policy implementation.
///
/// This implementation uses the built-in default policies from
/// [`CodecError::default_recovery_policy`] without any customisation.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultRecoveryPolicy;

impl RecoveryPolicyHook for DefaultRecoveryPolicy {}
