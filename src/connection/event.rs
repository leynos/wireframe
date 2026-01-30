//! Internal event types for the connection actor select loop.

use crate::response::WireframeError;

/// Events returned by [`ConnectionActor::next_event`][super::ConnectionActor::next_event].
///
/// Only `Debug` is derived because `WireframeError<E>` does not implement
/// `Clone` or `PartialEq`.
#[derive(Debug)]
pub(super) enum Event<F, E> {
    Shutdown,
    High(Option<F>),
    Low(Option<F>),
    /// Frames drained from the multi-packet response channel.
    /// Frames are forwarded in channel order after low-priority queues to
    /// preserve fairness and reuse the existing back-pressure.
    /// The actor emits the protocol terminator when the sender closes the
    /// channel so downstream observers see end-of-stream signalling.
    MultiPacket(Option<F>),
    Response(Option<Result<F, WireframeError<E>>>),
    Idle,
}
