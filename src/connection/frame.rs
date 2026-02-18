//! Frame processing and emission helpers.

use log::warn;

use super::ConnectionActor;
use crate::{
    app::{Packet, fragment_utils::fragment_packet},
    correlation::CorrelatableFrame,
    push::FrameLike,
};

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
{
    /// Emit a multi-packet frame with correlation stamping applied.
    pub(super) fn emit_multi_packet_frame(&mut self, frame: F, out: &mut Vec<F>) {
        let mut frame = frame;
        self.apply_multi_packet_correlation(&mut frame);
        self.process_frame_with_hooks_and_metrics(frame, out);
    }

    /// Apply correlation stamping to a multi-packet frame.
    pub(super) fn apply_multi_packet_correlation(&mut self, frame: &mut F) {
        let Some(ctx) = self.active_output.multi_packet_mut() else {
            // No channel is active, so there is nothing to stamp.
            return;
        };

        if !ctx.is_stamping_enabled() {
            return;
        }

        let correlation_id = ctx.correlation_id();
        frame.set_correlation_id(correlation_id);

        if let Some(expected) = correlation_id {
            debug_assert!(
                CorrelatableFrame::correlation_id(frame) == Some(expected)
                    || CorrelatableFrame::correlation_id(frame).is_none(),
                "multi-packet frame correlation mismatch: expected={:?}, got={:?}",
                Some(expected),
                CorrelatableFrame::correlation_id(frame),
            );
        } else {
            debug_assert!(
                CorrelatableFrame::correlation_id(frame).is_none(),
                "multi-packet frame correlation unexpectedly present: got={:?}",
                CorrelatableFrame::correlation_id(frame),
            );
        }
    }

    /// Apply protocol hooks and increment metrics before emitting a frame.
    ///
    /// # Examples
    ///
    /// ```text
    /// actor.process_frame_with_hooks_and_metrics(frame, &mut out);
    /// ```
    pub(super) fn process_frame_with_hooks_and_metrics(&mut self, frame: F, out: &mut Vec<F>)
    where
        F: Packet,
    {
        if let Some(fragmenter) = self.fragmenter.as_deref() {
            let fragmented = fragment_packet(fragmenter, frame);
            match fragmented {
                Ok(frames) => frames
                    .into_iter()
                    .for_each(|frame| self.push_frame(frame, out)),
                Err(err) => {
                    warn!(
                        "failed to fragment frame: connection_id={:?}, peer={:?}, error={err:?}",
                        self.connection_id, self.peer_addr,
                    );
                    crate::metrics::inc_handler_errors();
                }
            }
        } else {
            self.push_frame(frame, out);
        }
    }

    /// Push a single frame to output after applying hooks and metrics.
    pub(super) fn push_frame(&mut self, frame: F, out: &mut Vec<F>) {
        let mut frame = frame;
        self.hooks.before_send(&mut frame, &mut self.ctx);
        out.push(frame);
        crate::metrics::inc_frames(crate::metrics::Direction::Outbound);
    }
}
