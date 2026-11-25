//! Hooks for customizing codec error recovery behaviour.

use std::time::Duration;

use super::{CodecErrorContext, RecoveryPolicy};
use crate::codec::error::CodecError;

/// Hook trait for customizing codec error recovery behaviour.
///
/// Implementations can override default recovery policies based on
/// application-specific requirements or connection state.
///
/// # Default Implementation
///
/// The default implementation ([`DefaultRecoveryPolicy`]) delegates to
/// [`CodecError::default_recovery_policy`] for all errors.
///
/// # Installation
///
/// Install an implementation with
/// [`WireframeApp::with_recovery_hook`](crate::app::WireframeApp::with_recovery_hook).
/// The inbound connection loop then consults the hook for every decoder
/// failure on a live connection before it drops the frame, quarantines the
/// connection, or disconnects. Without an installed hook every decoder failure
/// disconnects the connection and the hook is never consulted.
///
/// # Consultation order
///
/// For each decoder failure the loop:
///
/// 1. Calls [`recovery_policy`](Self::recovery_policy) to select a policy.
/// 2. Escalates the selection to [`RecoveryPolicy::Disconnect`] when the failure is an end-of-file
///    error, when a non-disconnecting policy would discard a frame the decoder did not consume, or
///    when the selected [`RecoveryPolicy::Drop`] exceeds
///    [`RecoveryConfig::max_consecutive_drops`](crate::codec::RecoveryConfig::max_consecutive_drops).
/// 3. Resolves the pause through [`quarantine_duration`](Self::quarantine_duration) when the
///    applied policy is [`RecoveryPolicy::Quarantine`].
/// 4. Calls [`on_error`](Self::on_error) with the *applied* policy.
/// 5. Records `wireframe_codec_errors_total` once for the failure.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use wireframe::codec::{
///     CodecError,
///     CodecErrorContext,
///     FramingError,
///     RecoveryPolicy,
///     RecoveryPolicyHook,
/// };
///
/// /// Pause after a protocol violation, but block peers that oversize a frame.
/// struct PauseOnProtocolErrors;
///
/// impl RecoveryPolicyHook for PauseOnProtocolErrors {
///     fn recovery_policy(&self, error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
///         match error {
///             // A single malformed message may be a glitch worth resynchronizing after.
///             CodecError::Protocol(_) => RecoveryPolicy::Quarantine,
///             // No honest peer declares a frame beyond the configured maximum.
///             CodecError::Framing(FramingError::OversizedFrame { .. }) => {
///                 RecoveryPolicy::Disconnect
///             }
///             other => other.default_recovery_policy(),
///         }
///     }
///
///     fn quarantine_duration(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> Duration {
///         Duration::from_secs(5)
///     }
/// }
/// ```
pub trait RecoveryPolicyHook: Send + Sync {
    /// Determine the recovery policy for a codec error.
    ///
    /// The default implementation delegates to
    /// [`CodecError::default_recovery_policy`].
    fn recovery_policy(&self, error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        error.default_recovery_policy()
    }

    /// Returns the pause applied when [`RecoveryPolicy::Quarantine`] is
    /// selected.
    ///
    /// This value controls the pause for an installed hook; the
    /// [`RecoveryConfig::quarantine_duration`](crate::codec::RecoveryConfig::quarantine_duration)
    /// field only applies when no hook is installed.
    ///
    /// Default: 30 seconds.
    fn quarantine_duration(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> Duration {
        Duration::from_secs(30)
    }

    /// Called before applying the recovery policy.
    ///
    /// `policy` is the policy the connection loop is about to apply, which can
    /// differ from the value [`recovery_policy`](Self::recovery_policy)
    /// selected when a fallback escalates the failure. Use this hook for
    /// logging, application metrics, or state updates. The default
    /// implementation does nothing.
    fn on_error(&self, _error: &CodecError, _ctx: &CodecErrorContext, _policy: RecoveryPolicy) {}
}

/// Default recovery policy implementation.
///
/// This implementation uses the built-in default policies from
/// [`CodecError::default_recovery_policy`] without any customization.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultRecoveryPolicy;

impl RecoveryPolicyHook for DefaultRecoveryPolicy {}
