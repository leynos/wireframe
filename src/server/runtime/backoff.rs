//! Back-off configuration for the server accept loop.

use std::time::Duration;

/// Configuration for exponential back-off timing in the accept loop.
///
/// Controls retry behaviour when `accept()` calls fail on the server's TCP listener.
/// The back-off starts at `initial_delay` and doubles on each failure, capped at `max_delay`.
///
/// # Default Values
/// - `initial_delay`: 10 milliseconds
/// - `max_delay`: 1 second
///
/// # Invariants
/// - `initial_delay` must not exceed `max_delay`
/// - `initial_delay` must be at least 1 millisecond
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackoffConfig {
    /// Delay used for the first retry after an `accept()` failure.
    pub initial_delay: Duration,
    /// Maximum back-off delay once retries have increased exponentially.
    pub max_delay: Duration,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
        }
    }
}

impl BackoffConfig {
    /// Clamp delays to sane bounds and ensure `initial_delay <= max_delay`.
    ///
    /// This prevents accidental misconfiguration (for example, inverted or
    /// zero durations) before the values are used in the accept loop.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::server::BackoffConfig;
    ///
    /// let cfg = BackoffConfig {
    ///     initial_delay: Duration::from_millis(5),
    ///     max_delay: Duration::from_millis(1),
    /// };
    ///
    /// let normalized = cfg.normalized();
    /// assert_eq!(normalized.initial_delay, Duration::from_millis(1));
    /// assert_eq!(normalized.max_delay, Duration::from_millis(5));
    /// ```
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.initial_delay = self.initial_delay.max(Duration::from_millis(1));
        self.max_delay = self.max_delay.max(Duration::from_millis(1));
        if self.initial_delay > self.max_delay {
            std::mem::swap(&mut self.initial_delay, &mut self.max_delay);
        }
        self
    }
}
