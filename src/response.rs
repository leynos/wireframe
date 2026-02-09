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

use futures::{Stream, StreamExt, stream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

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
    /// Construct a bounded channel and wrap its receiver in a
    /// `Response::MultiPacket`.
    ///
    /// Returns the sending half of the channel alongside the response so a
    /// handler can spawn background tasks that stream frames. The channel is
    /// bounded: once `capacity` frames are buffered, additional `send`
    /// operations await until the connection actor drains the queue.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero. This mirrors the behaviour of
    /// [`tokio::sync::mpsc::channel`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tokio::spawn;
    /// use wireframe::Response;
    ///
    /// async fn stream_frames() -> (tokio::sync::mpsc::Sender<u8>, Response<u8>) {
    ///     let (sender, response) = Response::with_channel(8);
    ///
    ///     let mut producer = sender.clone();
    ///     spawn(async move {
    ///         for frame in 0..3u8 {
    ///             if producer.send(frame).await.is_err() {
    ///                 return;
    ///             }
    ///         }
    ///
    ///         drop(producer);
    ///     });
    ///
    ///     (sender, response)
    /// }
    /// ```
    #[must_use]
    pub fn with_channel(capacity: usize) -> (mpsc::Sender<F>, Response<F, E>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (sender, Response::MultiPacket(receiver))
    }

    /// Convert this response into a stream of frames.
    ///
    /// `Response::Vec` with no frames and `Response::Empty` produce an empty
    /// stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::TryStreamExt;
    /// use wireframe::Response;
    ///
    /// # async fn demo() {
    /// let (tx, rx) = tokio::sync::mpsc::channel(1);
    /// tx.send(1u8).await.expect("send");
    /// drop(tx);
    /// let resp: Response<u8, ()> = Response::MultiPacket(rx);
    /// let frames: Vec<u8> = resp
    ///     .into_stream()
    ///     .try_collect()
    ///     .await
    ///     .expect("stream error");
    /// assert_eq!(frames, vec![1]);
    /// # }
    /// ```
    #[must_use]
    pub fn into_stream(self) -> FrameStream<F, E> {
        match self {
            Response::Single(f) => {
                stream::once(async move { Ok::<F, WireframeError<E>>(f) }).boxed()
            }
            Response::Vec(frames) => stream::iter(frames.into_iter().map(Ok)).boxed(),
            Response::Stream(s) => s,
            Response::MultiPacket(rx) => ReceiverStream::new(rx).map(Ok).boxed(),
            Response::Empty => stream::empty().boxed(),
        }
    }
}

/// A generic error type for wireframe operations.
///
/// This enum distinguishes between transport-level I/O errors, protocol-defined
/// logical errors, and codec-layer errors with structured context.
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
    /// A codec-layer error with structured context.
    ///
    /// Codec errors are categorised by their origin: framing errors (wire-level
    /// frame boundary issues), protocol errors (semantic violations), I/O
    /// errors, or EOF conditions. See [`crate::codec::CodecError`] for details.
    Codec(crate::codec::CodecError),
}

impl<E> From<E> for WireframeError<E> {
    fn from(e: E) -> Self { WireframeError::Protocol(e) }
}

impl<E> WireframeError<E> {
    /// Convert an I/O error into a `WireframeError`.
    #[must_use]
    pub fn from_io(e: std::io::Error) -> Self { WireframeError::Io(e) }

    /// Convert a codec error into a `WireframeError`.
    #[must_use]
    pub fn from_codec(e: crate::codec::CodecError) -> Self { WireframeError::Codec(e) }

    /// Returns true if this error represents a clean connection close.
    ///
    /// A clean close occurs when the peer closes the connection at a frame
    /// boundary, indicating no data was lost. Delegates to
    /// [`CodecError::is_clean_close`](crate::codec::CodecError::is_clean_close).
    #[must_use]
    pub fn is_clean_close(&self) -> bool {
        matches!(self, WireframeError::Codec(e) if e.is_clean_close())
    }
}

impl<E: std::fmt::Debug> std::fmt::Display for WireframeError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireframeError::Io(e) => write!(f, "transport error: {e}"),
            WireframeError::Protocol(e) => write!(f, "protocol error: {e:?}"),
            WireframeError::Codec(e) => write!(f, "codec error: {e}"),
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
            WireframeError::Codec(e) => Some(e),
        }
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use rstest::{fixture, rstest};
    use tokio::sync::mpsc::{self, error::TrySendError};

    use super::*;

    #[fixture]
    fn single_capacity_channel() -> (mpsc::Sender<u8>, Response<u8, ()>) {
        Response::with_channel(1)
    }

    #[rstest]
    #[tokio::test]
    async fn with_channel_streams_frames_and_respects_capacity(
        single_capacity_channel: (mpsc::Sender<u8>, Response<u8, ()>),
    ) {
        let (sender, response) = single_capacity_channel;
        let Response::MultiPacket(mut rx) = response else {
            panic!("with_channel did not return a MultiPacket response");
        };

        sender.send(1).await.expect("send first frame");
        assert!(matches!(sender.try_send(2), Err(TrySendError::Full(2))));

        assert_eq!(rx.recv().await, Some(1));

        sender
            .send(3)
            .await
            .expect("send follow-up frame after draining");
        drop(sender);

        assert_eq!(rx.recv().await, Some(3));
        assert_eq!(rx.recv().await, None);
    }

    #[rstest]
    #[tokio::test]
    async fn with_channel_sender_errors_when_receiver_dropped(
        single_capacity_channel: (mpsc::Sender<u8>, Response<u8, ()>),
    ) {
        let (sender, response) = single_capacity_channel;
        drop(response);

        assert!(matches!(sender.try_send(7), Err(TrySendError::Closed(7))));
    }

    #[rstest]
    #[tokio::test]
    async fn with_channel_receiver_detects_sender_drop(
        single_capacity_channel: (mpsc::Sender<u8>, Response<u8, ()>),
    ) {
        let (sender, response) = single_capacity_channel;
        let Response::MultiPacket(mut rx) = response else {
            panic!("with_channel did not return a MultiPacket response");
        };

        drop(sender);

        assert_eq!(rx.recv().await, None);
    }
}
