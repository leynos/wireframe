//! Queue drain operations and fairness-aware helpers.

use tokio::sync::mpsc::error::TryRecvError;

use super::{ConnectionActor, multi_packet::MultiPacketTerminationReason, state::ActorState};
use crate::{app::Packet, correlation::CorrelatableFrame, push::FrameLike};

/// Context for drain operations containing mutable references to output and actor state.
pub(super) struct DrainContext<'a, F> {
    pub(super) out: &'a mut Vec<F>,
    pub(super) state: &'a mut ActorState,
}

/// Queue variants processed by the connection actor.
#[derive(Clone, Copy)]
pub(super) enum QueueKind {
    High,
    Low,
    Multi,
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
{
    /// Handle the result of polling the high-priority queue.
    pub(super) fn process_high(
        &mut self,
        res: Option<F>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) {
        self.process_queue(QueueKind::High, res, DrainContext { out, state });
    }

    /// Process a queue-backed source with shared low-priority semantics.
    pub(super) fn process_queue(
        &mut self,
        kind: QueueKind,
        res: Option<F>,
        ctx: DrainContext<'_, F>,
    ) {
        if let Some(frame) = res {
            self.forward_queue_frame(kind, frame, ctx);
        } else {
            let DrainContext { out, state } = ctx;
            self.handle_empty_queue(kind, state, out);
        }
    }

    /// Handle the result of polling the low-priority queue.
    pub(super) fn process_low(&mut self, res: Option<F>, state: &mut ActorState, out: &mut Vec<F>) {
        self.process_queue(QueueKind::Low, res, DrainContext { out, state });
    }

    /// Handle frames drained from the multi-packet channel.
    pub(super) fn process_multi_packet(
        &mut self,
        res: Option<F>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) {
        self.process_queue(QueueKind::Multi, res, DrainContext { out, state });
    }

    /// Update counters and opportunistically drain the low-priority and multi-packet queues.
    pub(super) fn after_high(&mut self, out: &mut Vec<F>, state: &mut ActorState) {
        self.fairness.record_high_priority();

        if !self.fairness.should_yield_to_low_priority() {
            return;
        }

        if self.try_opportunistic_drain(
            QueueKind::Low,
            DrainContext {
                out: &mut *out,
                state: &mut *state,
            },
        ) {
            return;
        }

        let _ = self.try_opportunistic_drain(
            QueueKind::Multi,
            DrainContext {
                out: &mut *out,
                state: &mut *state,
            },
        );
    }

    /// Try to opportunistically drain a queue-backed source when fairness allows.
    ///
    /// Returns `true` when a frame is forwarded to `out`.
    pub(super) fn try_opportunistic_drain(
        &mut self,
        kind: QueueKind,
        ctx: DrainContext<'_, F>,
    ) -> bool {
        let DrainContext { out, state } = ctx;
        match kind {
            QueueKind::High => {
                debug_assert!(false, "try_opportunistic_drain(High) should not be called");
                false
            }
            QueueKind::Low => self.try_opportunistic_low_drain(state, out),
            QueueKind::Multi => self.try_opportunistic_multi_drain(state, out),
        }
    }

    /// Reset counters after processing a low-priority frame.
    pub(super) fn after_low(&mut self) { self.fairness.reset(); }

    /// Common logic for handling closed receivers.
    pub(super) fn handle_closed_receiver(
        receiver: &mut Option<tokio::sync::mpsc::Receiver<F>>,
        state: &mut ActorState,
    ) {
        *receiver = None;
        state.mark_closed();
    }

    fn forward_queue_frame(&mut self, kind: QueueKind, frame: F, ctx: DrainContext<'_, F>) {
        let DrainContext { out, state } = ctx;
        if self.should_emit_multi_packet_frame(kind) {
            self.emit_multi_packet_frame(frame, out);
        } else {
            self.process_frame_with_hooks_and_metrics(frame, out);
        }

        match kind {
            QueueKind::High => self.after_high(out, state),
            QueueKind::Low | QueueKind::Multi => self.after_low(),
        }
    }

    fn should_emit_multi_packet_frame(&mut self, kind: QueueKind) -> bool {
        matches!(kind, QueueKind::Multi)
            && self
                .active_output
                .multi_packet_mut()
                .is_some_and(|ctx| ctx.is_stamping_enabled())
    }

    fn handle_empty_queue(&mut self, kind: QueueKind, state: &mut ActorState, out: &mut Vec<F>) {
        match kind {
            QueueKind::High => {
                Self::handle_closed_receiver(&mut self.high_rx, state);
                self.fairness.reset();
            }
            QueueKind::Low => {
                Self::handle_closed_receiver(&mut self.low_rx, state);
            }
            QueueKind::Multi => {
                self.handle_multi_packet_closed(MultiPacketTerminationReason::Drained, state, out);
            }
        }
    }

    fn try_opportunistic_low_drain(&mut self, state: &mut ActorState, out: &mut Vec<F>) -> bool {
        let Some(receiver) = self.low_rx.as_mut() else {
            return false;
        };

        match receiver.try_recv() {
            Ok(frame) => {
                self.process_frame_with_hooks_and_metrics(frame, out);
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

    fn try_opportunistic_multi_drain(&mut self, state: &mut ActorState, out: &mut Vec<F>) -> bool {
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
