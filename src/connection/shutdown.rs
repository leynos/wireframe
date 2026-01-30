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
        if self.multi_packet.is_active() {
            let correlation = self.multi_packet.correlation_id();
            self.log_multi_packet_closure(MultiPacketTerminationReason::Shutdown, correlation);
            if let Some(mut rx) = self.multi_packet.take_channel() {
                rx.close();
                state.mark_closed();
            }
            self.clear_multi_packet();
            self.hooks.on_command_end(&mut self.ctx);
        }

        if self.response.take().is_some() {
            state.mark_closed();
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
        let correlation = self.multi_packet.correlation_id();
        self.log_multi_packet_closure(reason, correlation);
        if let Some(mut receiver) = self.multi_packet.take_channel() {
            receiver.close();
        }
        state.mark_closed();
        if let Some(frame) = self.hooks.stream_end_frame(&mut self.ctx) {
            self.emit_multi_packet_frame(frame, out);
            self.after_low();
        }
        self.clear_multi_packet();
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
