//! Public configuration for pooled wireframe clients.

use std::time::Duration;

const DEFAULT_POOL_SIZE: usize = 4;
const DEFAULT_MAX_IN_FLIGHT_PER_SOCKET: usize = 1;
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// Configuration for a pooled wireframe client.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use wireframe::client::ClientPoolConfig;
///
/// let config = ClientPoolConfig::default()
///     .pool_size(2)
///     .max_in_flight_per_socket(3)
///     .idle_timeout(Duration::from_secs(30));
/// assert_eq!(config.pool_size_value(), 2);
/// assert_eq!(config.max_in_flight_per_socket_value(), 3);
/// assert_eq!(config.idle_timeout_value(), Duration::from_secs(30));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientPoolConfig {
    pool_size: usize,
    max_in_flight_per_socket: usize,
    idle_timeout: Duration,
}

impl Default for ClientPoolConfig {
    fn default() -> Self {
        Self {
            pool_size: DEFAULT_POOL_SIZE,
            max_in_flight_per_socket: DEFAULT_MAX_IN_FLIGHT_PER_SOCKET,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

impl ClientPoolConfig {
    /// Set the number of physical sockets maintained by the pool.
    ///
    /// Values below `1` are clamped to `1`.
    #[must_use]
    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = size.max(1);
        self
    }

    /// Set the admission limit per physical socket.
    ///
    /// Values below `1` are clamped to `1`.
    #[must_use]
    pub fn max_in_flight_per_socket(mut self, limit: usize) -> Self {
        self.max_in_flight_per_socket = limit.max(1);
        self
    }

    /// Set how long an idle socket stays warm before being recycled.
    ///
    /// Durations below `1 ms` are clamped to `1 ms`.
    #[must_use]
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout.max(Duration::from_millis(1));
        self
    }

    /// Return the configured number of physical sockets.
    #[must_use]
    pub const fn pool_size_value(&self) -> usize { self.pool_size }

    /// Return the configured per-socket admission limit.
    #[must_use]
    pub const fn max_in_flight_per_socket_value(&self) -> usize { self.max_in_flight_per_socket }

    /// Return the configured idle recycle timeout.
    #[must_use]
    pub const fn idle_timeout_value(&self) -> Duration { self.idle_timeout }
}
