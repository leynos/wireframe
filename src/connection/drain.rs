//! Queue drain operations and fairness-aware helpers.

use tokio::sync::mpsc::error::TryRecvError;

use super::{
    ConnectionActor,
    multi_packet::MultiPacketTerminationReason,
    pushes::PushKind,
    state::ActorState,
};
use crate::{app::Packet, correlation::CorrelatableFrame, push::FrameLike};

/// Mutable buffers and scheduling state advanced by one actor step.
///
/// Handlers read and write the source state, the response output, and the
/// queued pushes as a unit, while the caller decides when each buffer is
/// flushed. Bundling them keeps the per-event handlers to a readable
/// parameter list.
pub(super) struct ActorStep<'a, F, P> {
    /// Which output sources are still open.
    pub(super) state: &'a mut ActorState,
    /// Response-shaped frames produced by this step.
    pub(super) out: &'a mut Vec<F>,
    /// Push frames produced by this step, already composed.
    pub(super) pushes: &'a mut Vec<P>,
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Handle the result of polling the high-priority queue.
    pub(super) fn process_high(&mut self, res: Option<P>, step: &mut ActorStep<'_, F, P>) {
        self.process_push(PushKind::High, res, step);
    }

    /// Handle the result of polling the low-priority queue.
    pub(super) fn process_low(&mut self, res: Option<P>, step: &mut ActorStep<'_, F, P>) {
        self.process_push(PushKind::Low, res, step);
    }

    /// Route one push result through composition, fairness, and closure handling.
    fn process_push(&mut self, kind: PushKind, res: Option<P>, step: &mut ActorStep<'_, F, P>) {
        let Some(frame) = res else {
            self.handle_empty_push_lane(kind, step.state);
            return;
        };
        self.compose_push(frame, step.pushes);
        match kind {
            PushKind::High => self.after_high(step),
            PushKind::Low => self.after_low(),
        }
    }

    /// Handle frames drained from the multi-packet channel.
    pub(super) fn process_multi_packet(&mut self, res: Option<F>, step: &mut ActorStep<'_, F, P>) {
        match res {
            Some(frame) => {
                self.emit_multi_packet_frame(frame, step.out);
                self.after_low();
            }
            None => self.handle_empty_multi_packet(step.state, step.out),
        }
    }

    /// Update counters and opportunistically drain lower-priority sources.
    pub(super) fn after_high(&mut self, step: &mut ActorStep<'_, F, P>) {
        self.fairness.record_high_priority();

        if !self.fairness.should_yield_to_low_priority() {
            return;
        }

        if self.try_opportunistic_low_drain(step.state, step.pushes) {
            return;
        }

        let _ = self.try_opportunistic_multi_drain(step.state, step.out);
    }

    /// Reset burst history after low-priority or multi-packet frames.
    pub(super) const fn after_low(&mut self) { self.fairness.reset(); }

    /// Common logic for handling a closed receiver of any frame type.
    pub(super) fn handle_closed_receiver<Q>(
        receiver: &mut Option<tokio::sync::mpsc::Receiver<Q>>,
        state: &mut ActorState,
    ) {
        *receiver = None;
        state.mark_closed();
    }

    /// Mark a push lane closed when polling reports no further frames.
    fn handle_empty_push_lane(&mut self, kind: PushKind, state: &mut ActorState) {
        match kind {
            PushKind::High => {
                Self::handle_closed_receiver(&mut self.high_rx, state);
                self.fairness.reset();
            }
            PushKind::Low => Self::handle_closed_receiver(&mut self.low_rx, state),
        }
    }

    /// Mark the multi-packet source closed when polling reports no frames.
    fn handle_empty_multi_packet(&mut self, state: &mut ActorState, out: &mut Vec<F>) {
        self.handle_multi_packet_closed(MultiPacketTerminationReason::Drained, state, out);
    }

    /// Perform one non-blocking low-priority drain after a fairness yield.
    pub(super) fn try_opportunistic_low_drain(
        &mut self,
        state: &mut ActorState,
        pushes: &mut Vec<P>,
    ) -> bool {
        let Some(receiver) = self.low_rx.as_mut() else {
            return false;
        };

        match receiver.try_recv() {
            Ok(frame) => {
                self.compose_push(frame, pushes);
                self.after_low();
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                Self::handle_closed_receiver(&mut self.low_rx, state);
                false
            }
        }
    }

    /// Perform one non-blocking multi-packet drain after a fairness yield.
    pub(super) fn try_opportunistic_multi_drain(
        &mut self,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> bool {
        let Some(ctx) = self.active_output.multi_packet_mut() else {
            return false;
        };
        let Some(rx) = ctx.channel_mut() else {
            return false;
        };

        match rx.try_recv() {
            Ok(frame) => {
                self.emit_multi_packet_frame(frame, out);
                self.after_low();
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.handle_multi_packet_closed(
                    MultiPacketTerminationReason::Disconnected,
                    state,
                    out,
                );
                false
            }
        }
    }
}
