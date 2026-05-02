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
    pub(crate) steps: u8,
    pub(crate) high_priority_queued: bool,
    pub(crate) low_priority_queued: bool,
    pub(crate) fairness_allows_low: bool,
    pub(crate) active_output: ActiveOutput,
    pub(crate) shutdown_requested: bool,
    pub(crate) emitted_high_priority: bool,
    pub(crate) emitted_low_priority: bool,
    pub(crate) response_completed: bool,
    pub(crate) multi_packet_completed: bool,
    pub(crate) shutdown_during_output: bool,
    pub(crate) multi_packet_terminal_count: u8,
}

impl ConnectionState {
    pub(crate) fn is_output_idle(&self) -> bool { matches!(self.active_output, ActiveOutput::Idle) }

    pub(crate) fn can_install_output(&self) -> bool {
        !self.shutdown_requested && self.is_output_idle()
    }

    pub(crate) fn can_emit_low_priority(&self) -> bool {
        self.low_priority_queued && (!self.high_priority_queued || self.fairness_allows_low)
    }
}
