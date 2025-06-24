//! Response and error types for handler outputs.
//!
//! `Response` lets handlers return single frames, multiple frames or a
//! stream of frames. `WireframeError` distinguishes transport errors from
//! protocol errors when streaming.

use std::pin::Pin;

use futures::stream::Stream;

/// A type alias for a type-erased, dynamically dispatched stream of frames.
///
/// Each yielded item is a `Result` containing either a frame `F` or a
/// [`WireframeError`] describing why the stream failed.
pub type FrameStream<F, E> =
    Pin<Box<dyn Stream<Item = Result<F, WireframeError<E>>> + Send + 'static>>;

/// Represents the full response to a request.
pub enum Response<F, E> {
    /// A single frame reply.
    Single(F),
    /// An optimised list of frames.
    Vec(Vec<F>),
    /// A potentially unbounded stream of frames.
    Stream(FrameStream<F, E>),
    /// A response with no frames.
    Empty,
}

impl<F: std::fmt::Debug, E> std::fmt::Debug for Response<F, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Response::Single(frame) => f.debug_tuple("Single").field(frame).finish(),
            Response::Vec(v) => f.debug_tuple("Vec").field(v).finish(),
            Response::Stream(_) => f.write_str("Stream(..)"),
            Response::Empty => f.write_str("Empty"),
        }
    }
}

impl<F, E> From<F> for Response<F, E> {
    fn from(f: F) -> Self { Response::Single(f) }
}

impl<F, E> From<Vec<F>> for Response<F, E> {
    fn from(v: Vec<F>) -> Self { Response::Vec(v) }
}

/// A generic error type for wireframe operations.
#[derive(Debug)]
pub enum WireframeError<E> {
    /// An error in the underlying transport (e.g., socket closed).
    Io(std::io::Error),
    /// A protocol-defined logical error.
    Protocol(E),
}

impl<E> From<E> for WireframeError<E> {
    fn from(e: E) -> Self { WireframeError::Protocol(e) }
}

impl<E> WireframeError<E> {
    /// Convert an I/O error into a `WireframeError`.
    #[must_use]
    pub fn from_io(e: std::io::Error) -> Self { WireframeError::Io(e) }
}
