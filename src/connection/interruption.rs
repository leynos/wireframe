//! Classification of the events that end a response's unbounded wait.
//!
//! A response is driven without a deadline while the connection is healthy, so
//! only a shutdown signal or the loss of peer observation ends that wait. Both
//! then continue the response rather than truncating it, but they differ in
//! whether the actor may abandon it, so the decision lives here rather than
//! inline in the drive loop.
//!
//! Keeping it separate also keeps the policy testable without a transport, and
//! leaves `drive` a plain select loop over its sources.

use super::{ConnectionActor, drive::ResponseInterruption, event::Event};

impl<F, E> ConnectionActor<F, E> {
    /// Classify an event that ends the unbounded wait for a response.
    ///
    /// `None` means the event carries a frame and must be dispatched normally. A
    /// shutdown abandons the response only when this actor was configured to do
    /// so; a server connection instead keeps delivering a response its peer is
    /// reading, because ADR-014 requires it to drain across `stop()`. The
    /// bounded drain still guarantees release, because it renews the idle grace
    /// per event and abandons a producer or write that stops making progress.
    /// Losing peer observation always continues the response the same way, and
    /// the idle grace is what then releases a peer that has vanished.
    ///
    /// # Examples
    ///
    /// A server connection keeps its response when a stop arrives:
    ///
    /// ```ignore
    /// assert_eq!(
    ///     actor.response_interruption(&Event::Shutdown),
    ///     Some(ResponseInterruption::Bounded),
    /// );
    /// ```
    pub(super) const fn response_interruption(
        &self,
        event: &Event<F, E>,
    ) -> Option<ResponseInterruption> {
        match event {
            Event::Shutdown if self.abandon_response_on_shutdown => {
                Some(ResponseInterruption::Abandon)
            }
            Event::Shutdown | Event::PeerClosed => Some(ResponseInterruption::Bounded),
            Event::High(_)
            | Event::Low(_)
            | Event::MultiPacket(_)
            | Event::Response(_)
            | Event::Idle => None,
        }
    }
}
