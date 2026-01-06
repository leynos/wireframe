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

use std::{net::SocketAddr, time::Duration};

use super::error::CodecError;

/// Recovery policies for codec errors.
///
/// Each policy defines how the framework responds to a codec error.
///
/// # Default Behaviour
///
/// [`CodecError::default_recovery_policy`] returns the recommended policy for
/// each error type. Applications can override this via [`RecoveryPolicyHook`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecoveryPolicy {
    /// Discard the malformed frame and continue processing.
    ///
    /// This is the default for recoverable errors like oversized frames or
    /// protocol violations that affect only a single message. The connection
    /// remains open and can process subsequent frames.
    ///
    /// # When to Use
    ///
    /// - Oversized frames that exceed `max_frame_length`
    /// - Empty frames where non-empty is expected
    /// - Protocol-level errors (unknown message type, sequence violation)
    #[default]
    Drop,

    /// Pause the connection temporarily before retrying.
    ///
    /// The connection enters a quarantine state for a configurable duration.
    /// During quarantine, no frames are processed. After the timeout, normal
    /// processing resumes.
    ///
    /// # When to Use
    ///
    /// - Rate-limiting misbehaving clients
    /// - Temporary back-off after repeated errors
    /// - Giving time for upstream issues to resolve
    Quarantine,

    /// Terminate the connection immediately.
    ///
    /// The connection is closed without processing further frames. This is
    /// required for unrecoverable errors where the framing state is corrupted
    /// or the transport has failed.
    ///
    /// # When to Use
    ///
    /// - I/O errors (socket closed, write failed)
    /// - Invalid frame length encoding (framing state corrupted)
    /// - EOF conditions (connection ending)
    Disconnect,
}

/// Structured context for codec error logging and diagnostics.
///
/// This struct captures connection-specific information to include in
/// structured logs and metrics when codec errors occur.
///
/// # Examples
///
/// ```
/// use wireframe::codec::CodecErrorContext;
///
/// let ctx = CodecErrorContext::new()
///     .with_connection_id(42)
///     .with_correlation_id(123);
///
/// assert_eq!(ctx.connection_id, Some(42));
/// assert_eq!(ctx.correlation_id, Some(123));
/// ```
#[derive(Clone, Debug, Default)]
pub struct CodecErrorContext {
    /// Unique identifier for the connection.
    pub connection_id: Option<u64>,

    /// Remote peer address.
    pub peer_address: Option<SocketAddr>,

    /// Correlation identifier from the frame, if available.
    ///
    /// This helps attribute errors to specific requests when debugging.
    pub correlation_id: Option<u64>,

    /// Codec instance state for debugging.
    ///
    /// May include sequence numbers, bytes processed, or other codec-specific
    /// state information.
    pub codec_state: Option<String>,
}

impl CodecErrorContext {
    /// Create a new empty context.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Set the connection identifier.
    #[must_use]
    pub fn with_connection_id(mut self, id: u64) -> Self {
        self.connection_id = Some(id);
        self
    }

    /// Set the peer address.
    #[must_use]
    pub fn with_peer_address(mut self, addr: SocketAddr) -> Self {
        self.peer_address = Some(addr);
        self
    }

    /// Set the correlation identifier.
    #[must_use]
    pub fn with_correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set codec-specific state information.
    #[must_use]
    pub fn with_codec_state(mut self, state: impl Into<String>) -> Self {
        self.codec_state = Some(state.into());
        self
    }
}

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

/// Configuration for recovery policy behaviour.
///
/// Use this to configure global recovery settings on the application builder.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use wireframe::codec::RecoveryConfig;
///
/// let config = RecoveryConfig::default()
///     .max_consecutive_drops(5)
///     .quarantine_duration(Duration::from_secs(60))
///     .log_dropped_frames(true);
///
/// assert_eq!(config.max_consecutive_drops, 5);
/// ```
#[derive(Clone, Debug)]
pub struct RecoveryConfig {
    /// Maximum consecutive dropped frames before escalating to disconnect.
    ///
    /// When this threshold is exceeded, the recovery policy escalates from
    /// [`RecoveryPolicy::Drop`] to [`RecoveryPolicy::Disconnect`].
    ///
    /// Default: 10.
    pub max_consecutive_drops: u32,

    /// Default quarantine duration.
    ///
    /// Default: 30 seconds.
    pub quarantine_duration: Duration,

    /// Whether to log dropped frames at warn level.
    ///
    /// Default: true.
    pub log_dropped_frames: bool,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            max_consecutive_drops: 10,
            quarantine_duration: Duration::from_secs(30),
            log_dropped_frames: true,
        }
    }
}

impl RecoveryConfig {
    /// Set the maximum consecutive dropped frames before disconnect.
    #[must_use]
    pub fn max_consecutive_drops(mut self, count: u32) -> Self {
        self.max_consecutive_drops = count;
        self
    }

    /// Set the default quarantine duration.
    #[must_use]
    pub fn quarantine_duration(mut self, duration: Duration) -> Self {
        self.quarantine_duration = duration;
        self
    }

    /// Set whether to log dropped frames.
    #[must_use]
    pub fn log_dropped_frames(mut self, enabled: bool) -> Self {
        self.log_dropped_frames = enabled;
        self
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn recovery_policy_default_is_drop() {
        assert_eq!(RecoveryPolicy::default(), RecoveryPolicy::Drop);
    }

    #[test]
    fn context_builder_sets_fields() {
        let ctx = CodecErrorContext::new()
            .with_connection_id(42)
            .with_correlation_id(123)
            .with_codec_state("seq=5");

        assert_eq!(ctx.connection_id, Some(42));
        assert_eq!(ctx.correlation_id, Some(123));
        assert_eq!(ctx.codec_state, Some("seq=5".to_string()));
    }

    #[test]
    fn context_with_peer_address() {
        let addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid test address");
        let ctx = CodecErrorContext::new().with_peer_address(addr);
        assert_eq!(ctx.peer_address, Some(addr));
    }

    #[test]
    fn default_recovery_policy_delegates_to_error() {
        use super::super::error::{EofError, FramingError};

        let hook = DefaultRecoveryPolicy;
        let ctx = CodecErrorContext::new();

        // Check various error types
        let err = CodecError::Framing(FramingError::OversizedFrame { size: 100, max: 50 });
        assert_eq!(hook.recovery_policy(&err, &ctx), RecoveryPolicy::Drop);

        let err = CodecError::Io(io::Error::other("test"));
        assert_eq!(hook.recovery_policy(&err, &ctx), RecoveryPolicy::Disconnect);

        let err = CodecError::Eof(EofError::CleanClose);
        assert_eq!(hook.recovery_policy(&err, &ctx), RecoveryPolicy::Disconnect);
    }

    #[test]
    fn default_quarantine_duration_is_30_seconds() {
        let hook = DefaultRecoveryPolicy;
        let ctx = CodecErrorContext::new();
        let err = CodecError::Io(io::Error::other("test"));

        assert_eq!(
            hook.quarantine_duration(&err, &ctx),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn recovery_config_builder() {
        let config = RecoveryConfig::default()
            .max_consecutive_drops(5)
            .quarantine_duration(Duration::from_secs(60))
            .log_dropped_frames(false);

        assert_eq!(config.max_consecutive_drops, 5);
        assert_eq!(config.quarantine_duration, Duration::from_secs(60));
        assert!(!config.log_dropped_frames);
    }

    #[test]
    fn recovery_config_defaults() {
        let config = RecoveryConfig::default();

        assert_eq!(config.max_consecutive_drops, 10);
        assert_eq!(config.quarantine_duration, Duration::from_secs(30));
        assert!(config.log_dropped_frames);
    }
}
