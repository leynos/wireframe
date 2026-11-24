//! Streaming response handling for the connection actor.
//!
//! A streamed frame and its protocol terminator inherit the request's
//! correlation identifier, exactly as single, vector, and multi-packet
//! responses do, so a client matching replies by identifier sees one rule for
//! every response shape.

use log::warn;

use super::{ConnectionActor, state::ActorState};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    hooks::ConnectionProtocol,
    push::FrameLike,
    response::WireframeError,
};

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Handle the next frame or error from the streaming response.
    pub(super) fn process_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        let is_none = res.is_none();
        let produced = self.handle_response(res, state, out)?;
        if produced {
            self.after_low();
        }
        if is_none {
            self.active_output.clear_response();
        }
        Ok(())
    }

    /// Push a frame from the response stream into `out` or handle completion.
    ///
    /// Protocol errors are passed to `handle_error` and do not terminate the
    /// actor. I/O errors propagate to the caller.
    ///
    /// Returns `true` if a frame was appended to `out`.
    pub(super) fn handle_response(
        &mut self,
        res: Option<Result<F, WireframeError<E>>>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<bool, WireframeError<E>> {
        let mut produced = false;
        match res {
            Some(Ok(mut frame)) => {
                // Response frames inherit the request correlation on every
                // path; only the terminator's policy differs below.
                self.stamp_response_correlation(&mut frame);
                self.process_frame_with_hooks_and_metrics(frame, out);
                produced = true;
            }
            Some(Err(WireframeError::Protocol(e))) => {
                warn!("protocol error: error={e:?}");
                if let Some(protocol) = self.protocol.as_mut() {
                    protocol.handle_error(e);
                }
                state.mark_closed();
                // Stop polling the response after a protocol error to avoid
                // double-closing and duplicate `on_command_end` signalling.
                self.active_output.clear_response();
                self.response_failed = true;
                self.defer_command_end();
                crate::metrics::inc_handler_errors();
            }
            Some(Err(e)) => return Err(e),
            None => {
                state.mark_closed();
                let terminator = self
                    .protocol
                    .as_mut()
                    .and_then(ConnectionProtocol::stream_end_frame);
                if let Some(mut frame) = terminator {
                    self.terminator_correlation
                        .apply(&mut frame, self.response_correlation);
                    self.process_frame_with_hooks_and_metrics(frame, out);
                    produced = true;
                }
                // The terminator is still buffered, so the completion hook
                // must wait until the sink has written it.
                self.defer_command_end();
            }
        }

        Ok(produced)
    }

    /// Stamp the active response's request correlation onto a frame.
    ///
    /// Every response frame inherits the request's identifier when one was
    /// supplied; a frame that already carries its own keeps it when the
    /// request has none.
    fn stamp_response_correlation(&self, frame: &mut F) {
        let Some(correlation_id) = self.response_correlation else {
            return;
        };
        frame.set_correlation_id(Some(correlation_id));
    }
}
