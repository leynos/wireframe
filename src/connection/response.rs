//! Streaming response handling for the connection actor.

use log::warn;

use super::{ConnectionActor, state::ActorState};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    push::FrameLike,
    response::WireframeError,
};

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
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
            self.response = None;
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
            Some(Ok(frame)) => {
                self.process_frame_with_hooks_and_metrics(frame, out);
                produced = true;
            }
            Some(Err(WireframeError::Protocol(e))) => {
                warn!("protocol error: error={e:?}");
                self.hooks.handle_error(e, &mut self.ctx);
                state.mark_closed();
                // Stop polling the response after a protocol error to avoid
                // double-closing and duplicate `on_command_end` signalling.
                self.response = None;
                self.hooks.on_command_end(&mut self.ctx);
                crate::metrics::inc_handler_errors();
            }
            Some(Err(e)) => return Err(e),
            None => {
                state.mark_closed();
                if let Some(frame) = self.hooks.stream_end_frame(&mut self.ctx) {
                    self.process_frame_with_hooks_and_metrics(frame, out);
                    produced = true;
                }
                self.hooks.on_command_end(&mut self.ctx);
            }
        }

        Ok(produced)
    }
}
