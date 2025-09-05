//! Response and error types for handler outputs.
//!
//! `Response` lets handlers return single frames, multiple frames, multi-packet channels or a
//! stream of frames. `WireframeError` distinguishes transport errors from
//! protocol errors when streaming.
//!
//! # Examples
//!
//! ```
//! use futures::{StreamExt, stream};
//! use wireframe::Response;
//!
//! # type Frame = u8;
//! # type Error = ();
//! # fn make_frame(n: u8) -> Frame { n }
//!
//! // A single frame response
//! let single: Response<Frame, Error> = Response::Single(make_frame(1));
//!
//! // A vector of pre-built frames
//! let multiple: Response<Frame, Error> = Response::Vec(vec![make_frame(1), make_frame(2)]);
//!
//! // A streamed series of frames
//! let frames = stream::iter(vec![Ok(make_frame(1)), Ok(make_frame(2))])
//!     .map(|r: Result<Frame, Error>| r.map_err(wireframe::WireframeError::from));
//! let streamed: Response<Frame, Error> = Response::Stream(Box::pin(frames));
//!
//! // No-content response
//! let empty: Response<Frame, Error> = Response::Empty;
//! ```

use std::pin::Pin;

use futures::stream::Stream;
use tokio::sync::mpsc;

/// A type alias for a type-erased, dynamically dispatched stream of frames.
///
/// Each yielded item is a `Result` containing either a frame `F` or a
/// [`WireframeError`] describing why the stream failed.
pub type FrameStream<F, E = ()> =
    Pin<Box<dyn Stream<Item = Result<F, WireframeError<E>>> + Send + 'static>>;

/// Represents the full response to a request.
pub enum Response<F, E = ()> {
    /// A single frame reply.
    Single(F),
    /// An optimized list of frames.
    Vec(Vec<F>),
    /// A potentially unbounded stream of frames.
    Stream(FrameStream<F, E>),
    /// Frames delivered through a channel.
    ///
    /// # Usage and lifecycle
    ///
    /// `MultiPacket` wraps a [`tokio::sync::mpsc::Receiver`] that yields frames
    /// (`F`) sent from another task. The receiver should be polled until it
    /// returns `None`, signalling the channel has closed and no more frames will
    /// be sent. Frames are yielded in send order.
    /// Back-pressure follows the channel's capacity: senders await when it is
    /// full. Multiple senders may be cloned from the original `Sender`.
    /// The stream ends once all senders are dropped and `recv` returns
    /// `None`.
    ///
    /// # Resource management
    ///
    /// To avoid resource leaks or deadlocks:
    /// - Drop the sender once all frames are sent.
    /// - Poll the receiver to completion, consuming all frames.
    /// - Drop the receiver to cancel outstanding sends; subsequent sends fail.
    /// - If the sender is dropped early, the receiver yields `None` and no further frames will
    ///   arrive.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tokio::sync::mpsc;
    /// use wireframe::Response;
    ///
    /// async fn demo() {
    ///     let (tx, rx) = mpsc::channel(1);
    ///     tx.send(1u8).await.expect("send");
    ///     drop(tx); // close sender
    ///     if let Response::MultiPacket(mut rx) = Response::MultiPacket(rx) {
    ///         while let Some(f) = rx.recv().await {
    ///             assert_eq!(f, 1);
    ///         }
    ///     }
    /// }
    /// ```
    MultiPacket(mpsc::Receiver<F>),
    /// A response with no frames.
    Empty,
}

impl<F: std::fmt::Debug, E> std::fmt::Debug for Response<F, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Response::Single(frame) => f.debug_tuple("Single").field(frame).finish(),
            Response::Vec(v) => f.debug_tuple("Vec").field(v).finish(),
            Response::Stream(_) => f.write_str("Stream(..)"),
            Response::MultiPacket(_) => f.write_str("MultiPacket(..)"),
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

impl<F: Send + 'static, E: Send + 'static> Response<F, E> {
    /// Convert this response into a stream of frames.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::StreamExt;
    /// use wireframe::Response;
    ///
    /// # async fn demo() {
    /// let (tx, rx) = tokio::sync::mpsc::channel(1);
    /// tx.send(1u8).await.expect("send");
    /// drop(tx);
    /// let resp: Response<u8, ()> = Response::MultiPacket(rx);
    /// let frames: Vec<u8> = resp
    ///     .into_stream()
    ///     .map(|r| r.expect("stream error"))
    ///     .collect()
    ///     .await;
    /// assert_eq!(frames, vec![1]);
    /// # }
    /// ```
    #[must_use]
    pub fn into_stream(self) -> FrameStream<F, E> {
        match self {
            Response::Single(frame) => Box::pin(futures::stream::once(async move { Ok(frame) })),
            Response::Vec(frames) => Box::pin(futures::stream::iter(frames.into_iter().map(Ok))),
            Response::Stream(stream) => stream,
            Response::MultiPacket(rx) => Box::pin(futures::stream::unfold(rx, |mut rx| async {
                rx.recv().await.map(|f| (Ok(f), rx))
            })),
            Response::Empty => Box::pin(futures::stream::empty()),
        }
    }
}

/// A generic error type for wireframe operations.
///
/// # Examples
///
/// ```no_run
/// use wireframe::response::WireframeError;
///
/// #[derive(Debug)]
/// enum MyError {
///     BadRequest,
/// }
///
/// let proto_err: WireframeError<MyError> = MyError::BadRequest.into();
/// let io_err: WireframeError<MyError> = WireframeError::from_io(std::io::Error::other("boom"));
/// # drop(proto_err);
/// # drop(io_err);
/// ```
#[derive(Debug)]
pub enum WireframeError<E = ()> {
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

impl<E: std::fmt::Debug> std::fmt::Display for WireframeError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireframeError::Io(e) => write!(f, "transport error: {e}"),
            WireframeError::Protocol(e) => write!(f, "protocol error: {e:?}"),
        }
    }
}

impl<E> std::error::Error for WireframeError<E>
where
    E: std::fmt::Debug + std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WireframeError::Io(e) => Some(e),
            WireframeError::Protocol(e) => Some(e),
        }
    }
}
