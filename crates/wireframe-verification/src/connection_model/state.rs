//! State for the placeholder connection-actor model.

/// Active output tracked by the placeholder model.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ActiveOutput {
    /// No response or multi-packet output is currently active.
    #[default]
    Idle,
    /// A response stream is active.
    Response,
    /// A multi-packet stream is active.
    MultiPacket,
}

/// Model state for the placeholder connection-actor abstraction.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct ConnectionState {
    /// Number of transitions taken so far; bounded by `PlaceholderConnectionModel::max_steps`.
    pub(crate) steps: u8,
    /// A high-priority output request is waiting to be emitted.
    pub(crate) high_priority_queued: bool,
    /// A low-priority output request is waiting to be emitted.
    pub(crate) low_priority_queued: bool,
    /// Fairness policy currently permits a low-priority emission.
    pub(crate) fairness_allows_low: bool,
    /// Output stream currently held by the connection.
    pub(crate) active_output: ActiveOutput,
    /// Shutdown has been requested for this connection.
    pub(crate) shutdown_requested: bool,
    /// At least one high-priority frame has been emitted.
    pub(crate) emitted_high_priority: bool,
    /// At least one low-priority frame has been emitted.
    pub(crate) emitted_low_priority: bool,
    /// A response output stream has been completed.
    pub(crate) response_completed: bool,
    /// A multi-packet output stream has been completed.
    pub(crate) multi_packet_completed: bool,
    /// Shutdown was requested or applied while an output was active.
    pub(crate) shutdown_during_output: bool,
    /// Number of terminator frames appended to multi-packet streams.
    pub(crate) multi_packet_terminal_count: u8,
}

impl ConnectionState {
    /// Returns `true` when no output stream is currently active.
    pub(crate) fn is_output_idle(&self) -> bool { matches!(self.active_output, ActiveOutput::Idle) }

    /// Returns `true` when a new output stream may be installed (output idle and no shutdown
    /// pending).
    pub(crate) fn can_install_output(&self) -> bool {
        !self.shutdown_requested && self.is_output_idle()
    }

    /// Returns `true` when the fairness policy permits emitting the queued low-priority output.
    pub(crate) fn can_emit_low_priority(&self) -> bool {
        self.low_priority_queued && (!self.high_priority_queued || self.fairness_allows_low)
    }
}
