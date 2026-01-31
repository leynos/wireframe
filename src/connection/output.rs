//! Active output source types for the connection actor.

use super::multi_packet::MultiPacketContext;
use crate::response::FrameStream;

/// Active output source for the connection actor.
///
/// At most one output source can be active at a time. This enum makes the
/// mutual exclusion compile-time enforced rather than runtime-asserted.
pub(super) enum ActiveOutput<F, E> {
    /// No output source is active.
    None,
    /// A streaming response is active.
    Response(FrameStream<F, E>),
    /// A multi-packet channel is active.
    MultiPacket(MultiPacketContext<F>),
}

/// Result of shutting down an active output source.
pub(super) struct ShutdownResult {
    /// Correlation ID of the multi-packet context, if any.
    pub(super) correlation_id: Option<u64>,
    /// Whether the source should be marked as closed in `ActorState`.
    pub(super) source_closed: bool,
    /// Whether `on_command_end` hook should be called.
    pub(super) call_on_command_end: bool,
}

/// Result of closing an active multi-packet channel.
pub(super) struct MultiPacketCloseResult {
    /// Correlation ID of the multi-packet context, if any.
    pub(super) correlation_id: Option<u64>,
}

/// Availability flags for event sources polled by the connection actor.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Availability flags are a natural fit for booleans; no state machine needed"
)]
#[derive(Clone, Copy)]
pub(super) struct EventAvailability {
    pub(super) high: bool,
    pub(super) low: bool,
    pub(super) multi_packet: bool,
    pub(super) response: bool,
}

impl<F, E> ActiveOutput<F, E> {
    /// Returns `true` if a streaming response is active.
    pub(super) fn is_response(&self) -> bool { matches!(self, Self::Response(_)) }

    /// Returns `true` if a multi-packet channel is active.
    pub(super) fn is_multi_packet(&self) -> bool { matches!(self, Self::MultiPacket(_)) }

    /// Returns a mutable reference to the multi-packet context if active.
    pub(super) fn multi_packet_mut(&mut self) -> Option<&mut MultiPacketContext<F>> {
        match self {
            Self::MultiPacket(ctx) => Some(ctx),
            _ => Option::None,
        }
    }

    /// Clears the response stream, leaving `None` in its place.
    pub(super) fn clear_response(&mut self) {
        if matches!(self, Self::Response(_)) {
            *self = Self::None;
        }
    }

    /// Perform shutdown cleanup and return the result.
    ///
    /// This takes ownership of the active output, closes any receivers, and
    /// returns metadata needed by the caller to complete shutdown handling.
    pub(super) fn shutdown(&mut self) -> ShutdownResult {
        match std::mem::replace(self, Self::None) {
            Self::MultiPacket(mut ctx) => {
                let correlation_id = ctx.correlation_id();
                let source_closed = if let Some(rx) = ctx.channel_mut() {
                    rx.close();
                    true
                } else {
                    false
                };
                ShutdownResult {
                    correlation_id,
                    source_closed,
                    call_on_command_end: true,
                }
            }
            Self::Response(_) => ShutdownResult {
                correlation_id: None,
                source_closed: true,
                call_on_command_end: false,
            },
            Self::None => ShutdownResult {
                correlation_id: None,
                source_closed: false,
                call_on_command_end: false,
            },
        }
    }

    /// Begin closing a multi-packet channel.
    ///
    /// This closes the receiver and returns the correlation ID for logging,
    /// but does NOT clear the context yet. The caller must call `clear()` after
    /// emitting any terminator frames that need correlation IDs applied.
    pub(super) fn close_multi_packet(&mut self) -> MultiPacketCloseResult {
        let correlation_id = self.multi_packet_mut().and_then(|ctx| ctx.correlation_id());
        if let Self::MultiPacket(ctx) = self
            && let Some(rx) = ctx.channel_mut()
        {
            rx.close();
        }
        MultiPacketCloseResult { correlation_id }
    }

    /// Clear the active output to `None`.
    pub(super) fn clear(&mut self) { *self = Self::None; }
}
