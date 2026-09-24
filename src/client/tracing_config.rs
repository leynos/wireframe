//! Tracing configuration for wireframe client operations.
//!
//! [`TracingConfig`] controls which client operations emit tracing spans and
//! whether per-command elapsed-time events are recorded.

use tracing::Level;

use super::tracing_timing::{ClientOperation, ClientTimingFlags};

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
#[derive(Clone, Debug)]
pub struct TracingConfig {
    /// Span level for connection establishment.
    pub(crate) connect_level: Level,
    /// Span level for one-way sends.
    pub(crate) send_level: Level,
    /// Span level for one-way receives.
    pub(crate) receive_level: Level,
    /// Span level for request-response calls.
    pub(crate) call_level: Level,
    /// Span level for streaming calls.
    pub(crate) streaming_level: Level,
    /// Span level for orderly connection close.
    pub(crate) close_level: Level,
    /// Per-operation elapsed-time selection.
    timing: ClientTimingFlags,
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
            timing: ClientTimingFlags::default(),
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
    pub const fn with_connect_level(mut self, level: Level) -> Self {
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
    pub const fn with_connect_timing(mut self, enabled: bool) -> Self {
        self.timing = self.timing.with_enabled(ClientOperation::Connect, enabled);
        self
    }

    /// Set the tracing level for `send` and `send_envelope` operations.
    #[must_use]
    pub const fn with_send_level(mut self, level: Level) -> Self {
        self.send_level = level;
        self
    }

    /// Enable or disable per-command timing for `send` and `send_envelope`.
    #[must_use]
    pub const fn with_send_timing(mut self, enabled: bool) -> Self {
        self.timing = self.timing.with_enabled(ClientOperation::Send, enabled);
        self
    }

    /// Set the tracing level for `receive` and `receive_envelope` operations.
    #[must_use]
    pub const fn with_receive_level(mut self, level: Level) -> Self {
        self.receive_level = level;
        self
    }

    /// Enable or disable per-command timing for `receive` and
    /// `receive_envelope`.
    #[must_use]
    pub const fn with_receive_timing(mut self, enabled: bool) -> Self {
        self.timing = self.timing.with_enabled(ClientOperation::Receive, enabled);
        self
    }

    /// Set the tracing level for `call` and `call_correlated` operations.
    #[must_use]
    pub const fn with_call_level(mut self, level: Level) -> Self {
        self.call_level = level;
        self
    }

    /// Enable or disable per-command timing for `call` and `call_correlated`.
    #[must_use]
    pub const fn with_call_timing(mut self, enabled: bool) -> Self {
        self.timing = self.timing.with_enabled(ClientOperation::Call, enabled);
        self
    }

    /// Set the tracing level for `call_streaming` operations.
    #[must_use]
    pub const fn with_streaming_level(mut self, level: Level) -> Self {
        self.streaming_level = level;
        self
    }

    /// Enable or disable per-command timing for `call_streaming`.
    #[must_use]
    pub const fn with_streaming_timing(mut self, enabled: bool) -> Self {
        self.timing = self
            .timing
            .with_enabled(ClientOperation::Streaming, enabled);
        self
    }

    /// Set the tracing level for the `close` operation.
    #[must_use]
    pub const fn with_close_level(mut self, level: Level) -> Self {
        self.close_level = level;
        self
    }

    /// Enable or disable per-command timing for the `close` operation.
    #[must_use]
    pub const fn with_close_timing(mut self, enabled: bool) -> Self {
        self.timing = self.timing.with_enabled(ClientOperation::Close, enabled);
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
    pub const fn with_all_levels(mut self, level: Level) -> Self {
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
    pub const fn with_all_timing(mut self, enabled: bool) -> Self {
        self.timing = ClientTimingFlags::all(enabled);
        self
    }

    /// Report whether one client operation emits an elapsed-time event.
    #[must_use]
    pub(crate) const fn timing_enabled(&self, operation: ClientOperation) -> bool {
        self.timing.is_enabled(operation)
    }
}

#[cfg(test)]
mod tests {
    //! Verifies `TracingConfig` default values, bulk setters, and per-field
    //! setter isolation. Helpers `assert_levels` and `assert_timing_flags`
    //! compare all six operation slots in a single call.

    use rstest::rstest;
    use tracing::Level;

    use super::{ClientOperation, TracingConfig};

    /// Assert all level fields match the expected pattern.
    ///
    /// Order: connect, send, receive, call, streaming, close.
    fn assert_levels(
        cfg: &TracingConfig,
        [connect, send, receive, call, streaming, close]: [Level; 6],
    ) {
        assert_eq!(cfg.connect_level, connect, "connect_level");
        assert_eq!(cfg.send_level, send, "send_level");
        assert_eq!(cfg.receive_level, receive, "receive_level");
        assert_eq!(cfg.call_level, call, "call_level");
        assert_eq!(cfg.streaming_level, streaming, "streaming_level");
        assert_eq!(cfg.close_level, close, "close_level");
    }

    /// Assert all timing flags match the expected pattern.
    ///
    /// Order: connect, send, receive, call, streaming, close.
    #[track_caller]
    fn assert_timing_flags(cfg: &TracingConfig, expected: [bool; 6]) {
        let actual = [
            cfg.timing_enabled(ClientOperation::Connect),
            cfg.timing_enabled(ClientOperation::Send),
            cfg.timing_enabled(ClientOperation::Receive),
            cfg.timing_enabled(ClientOperation::Call),
            cfg.timing_enabled(ClientOperation::Streaming),
            cfg.timing_enabled(ClientOperation::Close),
        ];
        assert_eq!(
            actual, expected,
            "timing flags are ordered connect, send, receive, call, streaming, close"
        );
    }

    #[test]
    fn default_levels_are_info_for_lifecycle_debug_for_data_ops() {
        let cfg = TracingConfig::default();
        assert_levels(
            &cfg,
            [
                Level::INFO,
                Level::DEBUG,
                Level::DEBUG,
                Level::DEBUG,
                Level::DEBUG,
                Level::INFO,
            ],
        );
    }

    #[test]
    fn default_timing_flags_are_all_false() {
        let cfg = TracingConfig::default();
        assert_timing_flags(&cfg, [false; 6]);
    }

    #[test]
    fn with_all_levels_updates_every_level_field() {
        let cfg = TracingConfig::default().with_all_levels(Level::TRACE);
        assert_levels(&cfg, [Level::TRACE; 6]);
    }

    #[test]
    fn with_all_timing_true_enables_every_flag() {
        let cfg = TracingConfig::default().with_all_timing(true);
        assert_timing_flags(&cfg, [true; 6]);
    }

    #[test]
    fn with_all_timing_false_disables_every_flag() {
        let cfg = TracingConfig::default()
            .with_all_timing(true)
            .with_all_timing(false);
        assert_timing_flags(&cfg, [false; 6]);
    }

    #[test]
    fn disabling_one_timing_flag_preserves_other_enabled_operations() {
        let cfg = TracingConfig::default()
            .with_all_timing(true)
            .with_send_timing(false);
        assert_timing_flags(&cfg, [true, false, true, true, true, true]);
    }

    #[rstest]
    #[case::connect(
        TracingConfig::default().with_connect_timing(true),
        [true, false, false, false, false, false],
    )]
    #[case::send(
        TracingConfig::default().with_send_timing(true),
        [false, true, false, false, false, false],
    )]
    #[case::receive(
        TracingConfig::default().with_receive_timing(true),
        [false, false, true, false, false, false],
    )]
    #[case::call(
        TracingConfig::default().with_call_timing(true),
        [false, false, false, true, false, false],
    )]
    #[case::streaming(
        TracingConfig::default().with_streaming_timing(true),
        [false, false, false, false, true, false],
    )]
    #[case::close(
        TracingConfig::default().with_close_timing(true),
        [false, false, false, false, false, true],
    )]
    fn individual_timing_setters_only_affect_their_flag(
        #[case] cfg: TracingConfig,
        #[case] expected: [bool; 6],
    ) {
        assert_timing_flags(&cfg, expected);
    }

    #[rstest]
    #[case::connect(
        TracingConfig::default().with_connect_level(Level::TRACE),
        [Level::TRACE, Level::DEBUG, Level::DEBUG, Level::DEBUG, Level::DEBUG, Level::INFO],
    )]
    #[case::send(
        TracingConfig::default().with_send_level(Level::TRACE),
        [Level::INFO, Level::TRACE, Level::DEBUG, Level::DEBUG, Level::DEBUG, Level::INFO],
    )]
    #[case::receive(
        TracingConfig::default().with_receive_level(Level::TRACE),
        [Level::INFO, Level::DEBUG, Level::TRACE, Level::DEBUG, Level::DEBUG, Level::INFO],
    )]
    #[case::call(
        TracingConfig::default().with_call_level(Level::TRACE),
        [Level::INFO, Level::DEBUG, Level::DEBUG, Level::TRACE, Level::DEBUG, Level::INFO],
    )]
    #[case::streaming(
        TracingConfig::default().with_streaming_level(Level::TRACE),
        [Level::INFO, Level::DEBUG, Level::DEBUG, Level::DEBUG, Level::TRACE, Level::INFO],
    )]
    #[case::close(
        TracingConfig::default().with_close_level(Level::TRACE),
        [Level::INFO, Level::DEBUG, Level::DEBUG, Level::DEBUG, Level::DEBUG, Level::TRACE],
    )]
    fn individual_level_setters_only_affect_their_field(
        #[case] cfg: TracingConfig,
        #[case] expected: [Level; 6],
    ) {
        assert_levels(&cfg, expected);
    }
}
