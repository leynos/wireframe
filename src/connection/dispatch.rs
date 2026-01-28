//! Event dispatching for the connection actor.

use super::{ConnectionActor, event::Event, state::ActorState};
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
    /// Dispatch the given event to the appropriate handler.
    pub(super) fn dispatch_event(
        &mut self,
        event: Event<F, E>,
        state: &mut ActorState,
        out: &mut Vec<F>,
    ) -> Result<(), WireframeError<E>> {
        match event {
            Event::Shutdown => self.process_shutdown(state),
            Event::High(res) => self.process_high(res, state, out),
            Event::Low(res) => self.process_low(res, state, out),
            Event::MultiPacket(res) => self.process_multi_packet(res, state, out),
            Event::Response(res) => self.process_response(res, state, out)?,
            Event::Idle => {}
        }

        Ok(())
    }
}
