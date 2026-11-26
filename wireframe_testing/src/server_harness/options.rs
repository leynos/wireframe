//! Timeout policy for a running server handle.

use std::time::Duration;

/// Bound applied when no other value is configured.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout policy for a [`RunningWireframeServer`].
///
/// Both fields are optional so the pair helper, whose historical waits are
/// unbounded, can pass [`None`] for either one.
///
/// [`RunningWireframeServer`]: super::RunningWireframeServer
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use wireframe_testing::ServerHarnessOptions;
///
/// let options = ServerHarnessOptions::default()
///     .with_readiness_timeout(Some(Duration::from_millis(250)))
///     .with_shutdown_timeout(None);
/// assert_eq!(
///     options.readiness_timeout(),
///     Some(Duration::from_millis(250))
/// );
/// assert_eq!(options.shutdown_timeout(), None);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ServerHarnessOptions {
    /// Bound on waiting for the server's readiness signal.
    readiness_timeout: Option<Duration>,
    /// Bound on draining the server during an explicit shutdown.
    shutdown_timeout: Option<Duration>,
}

impl Default for ServerHarnessOptions {
    /// Five seconds to observe readiness and five seconds to drain on shutdown.
    fn default() -> Self {
        Self {
            readiness_timeout: Some(DEFAULT_TIMEOUT),
            shutdown_timeout: Some(DEFAULT_TIMEOUT),
        }
    }
}

impl ServerHarnessOptions {
    /// Replace the readiness bound.
    ///
    /// [`None`] waits indefinitely, which is what the pair helper needs.
    #[must_use]
    pub const fn with_readiness_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.readiness_timeout = timeout;
        self
    }

    /// Replace the shutdown bound.
    ///
    /// [`None`] waits indefinitely and matches `WireframePair::shutdown`.
    #[must_use]
    pub const fn with_shutdown_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// The readiness bound, or [`None`] when that wait is unbounded.
    #[must_use]
    pub const fn readiness_timeout(&self) -> Option<Duration> { self.readiness_timeout }

    /// The shutdown bound, or [`None`] when that drain is unbounded.
    #[must_use]
    pub const fn shutdown_timeout(&self) -> Option<Duration> { self.shutdown_timeout }
}
