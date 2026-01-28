//! Multi-packet channel state and correlation stamping.

use std::fmt;

use tokio::sync::mpsc;

/// Multi-packet correlation stamping state.
///
/// Tracks the active receiver and how frames should be stamped before emission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MultiPacketStamp {
    /// Stamping is disabled because no multi-packet channel is active.
    Disabled,
    /// Stamping is enabled and frames are stamped with the provided identifier.
    Enabled(Option<u64>),
}

/// Reasons why a multi-packet stream closed.
///
/// The reason informs logging severity so operators can distinguish between
/// natural completion and abrupt termination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MultiPacketTerminationReason {
    /// The sender dropped the channel after producing all frames.
    Drained,
    /// The sender was dropped without yielding an explicit end-of-stream.
    Disconnected,
    /// Shutdown cancelled the stream before it completed.
    Shutdown,
}

impl MultiPacketTerminationReason {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Drained => "drained",
            Self::Disconnected => "disconnected",
            Self::Shutdown => "shutdown",
        }
    }
}

impl fmt::Display for MultiPacketTerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
}

/// Multi-packet channel state tracking the active receiver and stamping config.
pub(super) struct MultiPacketContext<F> {
    channel: Option<mpsc::Receiver<F>>,
    stamp: MultiPacketStamp,
}

impl<F> MultiPacketContext<F> {
    pub(super) const fn new() -> Self {
        Self {
            channel: None,
            stamp: MultiPacketStamp::Disabled,
        }
    }

    /// Install a multi-packet channel with an optional correlation identifier.
    ///
    /// When `channel` is `Some`, stamping is enabled with the provided `correlation_id`.
    /// When `channel` is `None`, stamping is disabled and `correlation_id` is ignored.
    pub(super) fn install(
        &mut self,
        channel: Option<mpsc::Receiver<F>>,
        correlation_id: Option<u64>,
    ) {
        let stamp = if channel.is_some() {
            MultiPacketStamp::Enabled(correlation_id)
        } else {
            MultiPacketStamp::Disabled
        };
        self.channel = channel;
        self.stamp = stamp;
    }

    pub(super) fn clear(&mut self) {
        self.channel = None;
        self.stamp = MultiPacketStamp::Disabled;
    }

    pub(super) fn channel_mut(&mut self) -> Option<&mut mpsc::Receiver<F>> { self.channel.as_mut() }

    pub(super) fn take_channel(&mut self) -> Option<mpsc::Receiver<F>> { self.channel.take() }

    /// Returns `true` if correlation stamping is enabled.
    pub(super) fn is_stamping_enabled(&self) -> bool {
        matches!(self.stamp, MultiPacketStamp::Enabled(_))
    }

    pub(super) fn correlation_id(&self) -> Option<u64> {
        match self.stamp {
            MultiPacketStamp::Enabled(value) => value,
            MultiPacketStamp::Disabled => None,
        }
    }

    pub(super) fn is_active(&self) -> bool { self.channel.is_some() }
}
