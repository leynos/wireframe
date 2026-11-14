//! Event dispatching for the connection actor.

use super::{ConnectionActor, drain::ActorStep, event::Event};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    push::FrameLike,
    response::WireframeError,
};

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Dispatch the given event to the appropriate handler.
    ///
    /// Response-shaped output is buffered in `step.out` and queued pushes in
    /// `step.pushes`. Callers flush both buffers after every event, pushes
    /// first, so the wire order matches the order in which the sources
    /// produced them.
    pub(super) fn dispatch_event(
        &mut self,
        event: Event<F, E, P>,
        step: &mut ActorStep<'_, F, P>,
    ) -> Result<(), WireframeError<E>> {
        match event {
            Event::Shutdown => self.process_shutdown(step.state),
            Event::High(res) => self.process_high(res, step),
            Event::Low(res) => self.process_low(res, step),
            Event::MultiPacket(res) => self.process_multi_packet(res, step),
            Event::Response(res) => self.process_response(res, step.state, step.out)?,
            // Only the response drive observes a half-close; it handles the
            // event itself and never dispatches it.
            Event::Idle | Event::PeerClosed => {}
        }

        Ok(())
    }
}
