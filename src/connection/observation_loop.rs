//! Event observation for the connection actor.
//!
//! Every observation mode — the standalone drive, the prepared connection's
//! response drive, and the drain that follows a peer close — waits on the one
//! biased `tokio::select!` loop defined here. Keeping the loop apart from the
//! response driving that consumes its events leaves the priority ordering in a
//! single place for a refactor to preserve.

use super::{
    ConnectionActor,
    event::{Event, EventWatch},
    output::{AvailableOutput, EventAvailability},
    polling::EventSources,
    state::ActorState,
};
use crate::{app::Packet, correlation::CorrelatableFrame, push::FrameLike};

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Compute which event sources are currently available for polling.
    const fn compute_availability(&self, state: &ActorState) -> EventAvailability {
        EventAvailability {
            high: self.high_rx.is_some(),
            low: self.low_rx.is_some(),
            output: self
                .active_output
                .available_output(state.is_shutting_down()),
        }
    }

    /// Await the next ready event using biased priority ordering.
    ///
    /// Shutdown is observed first, followed by high-priority pushes, then
    /// low-priority pushes, multi-packet channels, and finally the response
    /// stream. This matches the design documentation. The final `else` branch
    /// prevents `tokio::select!` from panicking if all guards are false.
    ///
    /// The `strict_priority_order` and `shutdown_signal_precedence` tests
    /// assert that this ordering is preserved across refactors.
    pub(super) async fn next_event(&mut self, state: &ActorState) -> Event<F, E, P> {
        self.next_event_inner(
            state,
            EventWatch {
                peer: false,
                shutdown: true,
            },
        )
        .await
    }

    /// Whether peer-close observation applies to the current actor state.
    const fn should_watch_peer(watch: EventWatch, state: &ActorState) -> bool {
        watch.peer && state.is_active()
    }

    /// Shared select loop for every observation mode.
    pub(super) async fn next_event_inner(
        &mut self,
        state: &ActorState,
        watch: EventWatch,
    ) -> Event<F, E, P> {
        let avail = self.compute_availability(state);
        let can_poll_multi_packet = avail.output == AvailableOutput::MultiPacket;
        let can_poll_response = avail.output == AvailableOutput::Response;
        let EventSources {
            high,
            low,
            multi_packet,
            response,
            shutdown,
            peer_closed,
        } = self.event_sources();

        tokio::select! {
            biased;

            () = Self::wait_shutdown(shutdown), if watch.shutdown && state.is_active() => {
                Event::Shutdown
            }
            () = Self::wait_peer_closed(peer_closed), if Self::should_watch_peer(watch, state) => {
                Event::PeerClosed
            }
            res = Self::poll_queue(high), if avail.high => Event::High(res),
            res = Self::poll_queue(low), if avail.low => Event::Low(res),
            res = Self::poll_queue(multi_packet), if can_poll_multi_packet => Event::MultiPacket(res),
            res = Self::poll_response(response), if can_poll_response => Event::Response(res),
            else => Event::Idle,
        }
    }
}
