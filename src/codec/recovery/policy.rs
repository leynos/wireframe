//! Recovery policies for codec errors.

/// Recovery policies for codec errors.
///
/// Each policy defines how the framework responds to a codec error.
///
/// # Default Behaviour
///
/// [`CodecError::default_recovery_policy`](crate::codec::CodecError::default_recovery_policy)
/// returns the recommended policy for each error type. Applications can
/// override this via [`RecoveryPolicyHook`](crate::codec::RecoveryPolicyHook).
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

impl RecoveryPolicy {
    /// Returns the policy name as a static string for metrics and logging.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::codec::RecoveryPolicy;
    ///
    /// assert_eq!(RecoveryPolicy::Drop.as_str(), "drop");
    /// assert_eq!(RecoveryPolicy::Quarantine.as_str(), "quarantine");
    /// assert_eq!(RecoveryPolicy::Disconnect.as_str(), "disconnect");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Drop => "drop",
            Self::Quarantine => "quarantine",
            Self::Disconnect => "disconnect",
        }
    }
}
