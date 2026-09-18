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
