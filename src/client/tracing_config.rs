//! Tracing configuration for wireframe client operations.
//!
//! [`TracingConfig`] controls which client operations emit tracing spans and
//! whether per-command elapsed-time events are recorded.

use tracing::Level;

/// Controls tracing span levels and per-command timing for client operations.
///
/// By default, lifecycle operations (`connect`, `close`) emit spans at
/// `INFO` level. High-frequency operations (`send`, `receive`, `call`,
/// `call_streaming`) emit spans at `DEBUG` level. Per-command timing is
/// disabled for all operations by default.
///
/// Spans are always created at the configured level. When no `tracing`
/// subscriber is installed, span creation is a no-op (zero-cost). When
/// per-command timing is enabled for an operation, an additional event
/// recording `elapsed_us` is emitted when the operation completes.
///
/// # Examples
///
/// ```
/// use tracing::Level;
/// use wireframe::client::TracingConfig;
///
/// // Enable timing for connect and call operations only.
/// let config = TracingConfig::default()
///     .with_connect_timing(true)
///     .with_call_timing(true);
/// let _ = config;
///
/// // Set all operations to TRACE level.
/// let verbose = TracingConfig::default()
///     .with_all_levels(Level::TRACE)
///     .with_all_timing(true);
/// let _ = verbose;
/// ```
#[expect(
    clippy::struct_excessive_bools,
    reason = "six independent on/off timing flags — one per operation category"
)]
#[derive(Clone, Debug)]
pub struct TracingConfig {
    pub(crate) connect_level: Level,
    pub(crate) send_level: Level,
    pub(crate) receive_level: Level,
    pub(crate) call_level: Level,
    pub(crate) streaming_level: Level,
    pub(crate) close_level: Level,
    pub(crate) connect_timing: bool,
    pub(crate) send_timing: bool,
    pub(crate) receive_timing: bool,
    pub(crate) call_timing: bool,
    pub(crate) streaming_timing: bool,
    pub(crate) close_timing: bool,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            connect_level: Level::INFO,
            send_level: Level::DEBUG,
            receive_level: Level::DEBUG,
            call_level: Level::DEBUG,
            streaming_level: Level::DEBUG,
            close_level: Level::INFO,
            connect_timing: false,
            send_timing: false,
            receive_timing: false,
            call_timing: false,
            streaming_timing: false,
            close_timing: false,
        }
    }
}

impl TracingConfig {
    /// Set the tracing level for the `connect` operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use tracing::Level;
    /// use wireframe::client::TracingConfig;
    ///
    /// let config = TracingConfig::default().with_connect_level(Level::TRACE);
    /// let _ = config;
    /// ```
    #[must_use]
    pub fn with_connect_level(mut self, level: Level) -> Self {
        self.connect_level = level;
        self
    }

    /// Enable or disable per-command timing for the `connect` operation.
    ///
    /// When enabled, an event recording `elapsed_us` is emitted at `DEBUG`
    /// level when the connect operation completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::TracingConfig;
    ///
    /// let config = TracingConfig::default().with_connect_timing(true);
    /// let _ = config;
    /// ```
    #[must_use]
    pub fn with_connect_timing(mut self, enabled: bool) -> Self {
        self.connect_timing = enabled;
        self
    }

    /// Set the tracing level for `send` and `send_envelope` operations.
    #[must_use]
    pub fn with_send_level(mut self, level: Level) -> Self {
        self.send_level = level;
        self
    }

    /// Enable or disable per-command timing for `send` and `send_envelope`.
    #[must_use]
    pub fn with_send_timing(mut self, enabled: bool) -> Self {
        self.send_timing = enabled;
        self
    }

    /// Set the tracing level for `receive` and `receive_envelope` operations.
    #[must_use]
    pub fn with_receive_level(mut self, level: Level) -> Self {
        self.receive_level = level;
        self
    }

    /// Enable or disable per-command timing for `receive` and
    /// `receive_envelope`.
    #[must_use]
    pub fn with_receive_timing(mut self, enabled: bool) -> Self {
        self.receive_timing = enabled;
        self
    }

    /// Set the tracing level for `call` and `call_correlated` operations.
    #[must_use]
    pub fn with_call_level(mut self, level: Level) -> Self {
        self.call_level = level;
        self
    }

    /// Enable or disable per-command timing for `call` and `call_correlated`.
    #[must_use]
    pub fn with_call_timing(mut self, enabled: bool) -> Self {
        self.call_timing = enabled;
        self
    }

    /// Set the tracing level for `call_streaming` operations.
    #[must_use]
    pub fn with_streaming_level(mut self, level: Level) -> Self {
        self.streaming_level = level;
        self
    }

    /// Enable or disable per-command timing for `call_streaming`.
    #[must_use]
    pub fn with_streaming_timing(mut self, enabled: bool) -> Self {
        self.streaming_timing = enabled;
        self
    }

    /// Set the tracing level for the `close` operation.
    #[must_use]
    pub fn with_close_level(mut self, level: Level) -> Self {
        self.close_level = level;
        self
    }

    /// Enable or disable per-command timing for the `close` operation.
    #[must_use]
    pub fn with_close_timing(mut self, enabled: bool) -> Self {
        self.close_timing = enabled;
        self
    }

    /// Set the tracing level for all operations at once.
    ///
    /// # Examples
    ///
    /// ```
    /// use tracing::Level;
    /// use wireframe::client::TracingConfig;
    ///
    /// let config = TracingConfig::default().with_all_levels(Level::TRACE);
    /// let _ = config;
    /// ```
    #[must_use]
    pub fn with_all_levels(mut self, level: Level) -> Self {
        self.connect_level = level;
        self.send_level = level;
        self.receive_level = level;
        self.call_level = level;
        self.streaming_level = level;
        self.close_level = level;
        self
    }

    /// Enable or disable per-command timing for all operations at once.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::TracingConfig;
    ///
    /// let config = TracingConfig::default().with_all_timing(true);
    /// let _ = config;
    /// ```
    #[must_use]
    pub fn with_all_timing(mut self, enabled: bool) -> Self {
        self.connect_timing = enabled;
        self.send_timing = enabled;
        self.receive_timing = enabled;
        self.call_timing = enabled;
        self.streaming_timing = enabled;
        self.close_timing = enabled;
        self
    }
}
