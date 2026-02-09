//! Configuration for codec recovery policies.

use std::time::Duration;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryConfig {
    /// Maximum consecutive dropped frames before escalating to disconnect.
    ///
    /// When this threshold is exceeded, the recovery policy escalates from
    /// [`RecoveryPolicy::Drop`](crate::codec::RecoveryPolicy::Drop) to
    /// [`RecoveryPolicy::Disconnect`](crate::codec::RecoveryPolicy::Disconnect).
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
