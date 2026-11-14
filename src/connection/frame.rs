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
    pub(super) fn emit_multi_packet_frame(&mut self, mut frame: F, out: &mut Vec<F>) {
        self.apply_multi_packet_correlation(&mut frame);
        self.process_frame_with_hooks_and_metrics(frame, out);
    }

    /// Apply correlation stamping to a multi-packet frame.
    ///
    /// Stamping only replaces the frame's identifier when the originating
    /// request supplied one. A request without a correlation identifier
    /// leaves the producer's own identifier untouched, matching the
    /// inheritance rule applied to every other response shape.
    pub(super) fn apply_multi_packet_correlation(&mut self, frame: &mut F) {
        let Some(ctx) = self.active_output.multi_packet_mut() else {
            // No channel is active, so there is nothing to stamp.
            return;
        };

        if !ctx.is_stamping_enabled() {
            return;
        }

        // A request that supplied no correlation identifier must not clear an
        // identifier produced by the producer, matching the inheritance rule
        // used for single, vector, and streamed responses.
        let Some(correlation_id) = ctx.correlation_id() else {
            return;
        };
        frame.set_correlation_id(Some(correlation_id));

        debug_assert!(
            CorrelatableFrame::correlation_id(frame) == Some(correlation_id)
                || CorrelatableFrame::correlation_id(frame).is_none(),
            "multi-packet frame correlation mismatch: expected={correlation_id:?}, got={:?}",
            CorrelatableFrame::correlation_id(frame),
        );
    }

    /// Apply fragmentation and protocol hooks before emitting a frame.
    ///
    /// Outbound frame accounting happens in the sink that eventually writes
    /// the frame, so this method only composes it.
    ///
    /// # Examples
    ///
    /// ```plaintext
    /// actor.process_frame_with_hooks_and_metrics(frame, &mut out);
    /// ```
    pub(super) fn process_frame_with_hooks_and_metrics(&mut self, frame: F, out: &mut Vec<F>)
    where
        F: Packet,
    {
        if let Some(fragmenter) = self.fragmenter.as_ref() {
            let fragmented = fragment_packet(fragmenter, frame);
            match fragmented {
                Ok(frames) => frames
                    .into_iter()
                    .for_each(|fragment| self.push_frame(fragment, out)),
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

    /// Push a single frame to the current output buffer after applying hooks.
    pub(super) fn push_frame(&mut self, mut frame: F, out: &mut Vec<F>) {
        self.hooks.before_send(&mut frame, &mut self.ctx);
        out.push(frame);
    }
}
