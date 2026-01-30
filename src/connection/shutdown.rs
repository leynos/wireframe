//! Shutdown handling for the connection actor.

use log::info;

use super::{ConnectionActor, multi_packet::MultiPacketTerminationReason, state::ActorState};
use crate::{app::Packet, correlation::CorrelatableFrame, push::FrameLike};

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
{
    /// Begin shutdown once cancellation has been observed.
    pub(super) fn process_shutdown(&mut self, state: &mut ActorState) {
        state.start_shutdown();
        self.start_shutdown(state);
    }

    /// Close all receivers and mark streaming sources as closed if present.
    pub(super) fn start_shutdown(&mut self, state: &mut ActorState) {
        if let Some(rx) = &mut self.high_rx {
            rx.close();
        }
        if let Some(rx) = &mut self.low_rx {
            rx.close();
        }

        // Check if multi-packet is active before shutdown clears it
        let was_multi_packet = self.active_output.is_multi_packet();
        let result = self.active_output.shutdown();

        if result.source_closed {
            state.mark_closed();
        }
        if was_multi_packet {
            self.log_multi_packet_closure(
                MultiPacketTerminationReason::Shutdown,
                result.correlation_id,
            );
        }
        if result.call_on_command_end {
            self.hooks.on_command_end(&mut self.ctx);
        }
    }

    /// Handle a closed multi-packet channel by emitting the protocol terminator
    /// and notifying hooks.
    pub(super) fn handle_multi_packet_closed(
        &mut self,
        reason: MultiPacketTerminationReason,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) {
        let result = self.active_output.close_multi_packet();
        self.log_multi_packet_closure(reason, result.correlation_id);
        state.mark_closed();
        if let Some(frame) = self.hooks.stream_end_frame(&mut self.ctx) {
            self.emit_multi_packet_frame(frame, out);
            self.after_low();
        }
        self.active_output.clear();
        self.hooks.on_command_end(&mut self.ctx);
    }

    pub(super) fn log_multi_packet_closure(
        &self,
        reason: MultiPacketTerminationReason,
        correlation_id: Option<u64>,
    ) {
        use log::warn;
        let message = "multi-packet stream closed";
        match reason {
            MultiPacketTerminationReason::Disconnected => warn!(
                "{message}: reason={}, connection_id={:?}, peer={:?}, correlation_id={:?}",
                reason, self.connection_id, self.peer_addr, correlation_id,
            ),
            _ => info!(
                "{message}: reason={}, connection_id={:?}, peer={:?}, correlation_id={:?}",
                reason, self.connection_id, self.peer_addr, correlation_id,
            ),
        }
    }
}
