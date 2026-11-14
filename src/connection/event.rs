//! Internal event types for the connection actor select loop.

use crate::response::WireframeError;

/// Events returned by [`ConnectionActor::next_event`][super::ConnectionActor::next_event].
///
/// Only `Debug` is derived because `WireframeError<E>` does not implement
/// `Clone` or `PartialEq`.
#[derive(Debug)]
pub(super) enum Event<F, E> {
    /// Cancellation was observed and all sources must begin shutdown.
    Shutdown,
    /// The peer closed its input half while a response was still in flight.
    ///
    /// A peer that only half-closed may still be reading, so the response is
    /// not truncated immediately; see the bounded idle grace in `drive`.
    PeerClosed,
    /// Result from polling the urgent push queue.
    High(Option<F>),
    /// Result from polling the best-effort push queue.
    Low(Option<F>),
    /// Frames drained from the multi-packet response channel.
    /// Frames are forwarded in channel order after low-priority queues to
    /// preserve fairness and reuse the existing back-pressure.
    /// The actor emits the protocol terminator after the sender closes the
    /// channel only when `stream_end_frame` returns `Some`, so downstream
    /// observers see end-of-stream signalling when the hook supplies it.
    MultiPacket(Option<F>),
    /// Result from polling the streaming response, including transport errors.
    Response(Option<Result<F, WireframeError<E>>>),
    /// No source was eligible; this is a defensive select-loop fallback.
    Idle,
}

/// Which lifecycle signals one event wait observes.
///
/// The standalone driver always observes shutdown and never the peer. The
/// prepared connection observes the peer, and follows the actor's
/// `abandon_response_on_shutdown` policy for shutdown, so the two modes differ
/// in both flags and are grouped rather than passed separately.
#[derive(Clone, Copy, Debug)]
pub(super) struct EventWatch {
    /// Whether a peer half-close wakes the wait.
    pub(super) peer: bool,
    /// Whether a cancelled shutdown token ends the wait.
    pub(super) shutdown: bool,
}
