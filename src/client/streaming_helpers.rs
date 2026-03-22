//! Helpers for adapting streaming response frames into domain items.
//!
//! This module layers on top of [`super::ResponseStream`] and other compatible
//! streams without changing transport semantics, terminator handling, or the
//! exclusive client borrow held by the underlying response stream.

use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;

use super::ClientError;

/// Extension methods for adapting streaming response frames into typed items.
///
/// The helper keeps the underlying transport semantics intact:
///
/// - yielded items preserve their original order;
/// - `Ok(None)` mapper results skip control frames;
/// - mapper failures stop the stream immediately;
/// - underlying [`ClientError`] values are forwarded unchanged; and
/// - termination still occurs when the wrapped stream returns `None`.
///
/// This is useful when a protocol multiplexes data frames with notices,
/// progress updates, or other control packets that should not appear in the
/// final consumer-facing stream.
///
/// # Examples
///
/// ```rust,no_run
/// use std::net::SocketAddr;
///
/// use futures::TryStreamExt;
/// use wireframe::{
///     app::{Packet, PacketParts},
///     client::{ClientError, StreamingResponseExt, WireframeClient},
///     correlation::CorrelatableFrame,
/// };
///
/// #[derive(bincode::BorrowDecode, bincode::Encode, Debug)]
/// struct MyEnvelope {
///     id: u32,
///     correlation_id: Option<u64>,
///     payload: Vec<u8>,
/// }
///
/// impl CorrelatableFrame for MyEnvelope {
///     fn correlation_id(&self) -> Option<u64> { self.correlation_id }
///
///     fn set_correlation_id(&mut self, cid: Option<u64>) { self.correlation_id = cid; }
/// }
///
/// impl Packet for MyEnvelope {
///     fn id(&self) -> u32 { self.id }
///
///     fn into_parts(self) -> PacketParts {
///         PacketParts::new(self.id, self.correlation_id, self.payload)
///     }
///
///     fn from_parts(parts: PacketParts) -> Self {
///         Self {
///             id: parts.id(),
///             correlation_id: parts.correlation_id(),
///             payload: parts.into_payload(),
///         }
///     }
///
///     fn is_stream_terminator(&self) -> bool { self.id == 0 }
/// }
///
/// #[derive(Debug, PartialEq, Eq)]
/// struct Row(Vec<u8>);
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), ClientError> {
/// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
/// let mut client = WireframeClient::builder().connect(addr).await?;
///
/// let request = MyEnvelope {
///     id: 1,
///     correlation_id: None,
///     payload: vec![],
/// };
///
/// let rows: Vec<Row> = client
///     .call_streaming::<MyEnvelope>(request)
///     .await?
///     .typed_with(|frame| match frame.id {
///         1 => Ok(Some(Row(frame.payload))),
///         2 => Ok(None),
///         other => Err(ClientError::from(std::io::Error::new(
///             std::io::ErrorKind::InvalidData,
///             format!("unexpected frame id {other}"),
///         ))),
///     })
///     .try_collect()
///     .await?;
///
/// assert!(!rows.is_empty());
/// # Ok(())
/// # }
/// ```
pub trait StreamingResponseExt<P>: Stream<Item = Result<P, ClientError>> + Sized {
    /// Adapt protocol frames into typed items, skipping control frames when
    /// the mapper returns `Ok(None)`.
    #[must_use]
    fn typed_with<Item, Mapper>(self, mapper: Mapper) -> TypedResponseStream<Self, Mapper, P, Item>
    where
        Self: Unpin,
        Mapper: FnMut(P) -> Result<Option<Item>, ClientError> + Unpin,
    {
        TypedResponseStream::new(self, mapper)
    }
}

impl<S, P> StreamingResponseExt<P> for S where S: Stream<Item = Result<P, ClientError>> + Sized {}

/// Stream adapter that maps protocol frames into domain items.
///
/// Construct this via [`StreamingResponseExt::typed_with`].
///
/// # Examples
///
/// ```rust
/// use futures::{StreamExt, TryStreamExt, stream};
/// use wireframe::client::StreamingResponseExt;
///
/// # async fn demo() -> Result<(), wireframe::client::ClientError> {
/// let items: Vec<u8> = stream::iter(vec![Ok::<u8, _>(1), Ok(2), Ok(3)])
///     .typed_with(|frame| {
///         if frame % 2 == 0 {
///             Ok(None)
///         } else {
///             Ok(Some(frame))
///         }
///     })
///     .try_collect()
///     .await?;
///
/// assert_eq!(items, vec![1, 3]);
/// # Ok(())
/// # }
/// ```
pub struct TypedResponseStream<S, Mapper, P, Item>
where
    S: Stream<Item = Result<P, ClientError>>,
{
    inner: S,
    mapper: Mapper,
    _phantom: PhantomData<fn() -> (P, Item)>,
}

impl<S, Mapper, P, Item> TypedResponseStream<S, Mapper, P, Item>
where
    S: Stream<Item = Result<P, ClientError>>,
{
    fn new(inner: S, mapper: Mapper) -> Self {
        Self {
            inner,
            mapper,
            _phantom: PhantomData,
        }
    }
}

impl<S, Mapper, P, Item> Stream for TypedResponseStream<S, Mapper, P, Item>
where
    S: Stream<Item = Result<P, ClientError>> + Unpin,
    Mapper: FnMut(P) -> Result<Option<Item>, ClientError> + Unpin,
{
    type Item = Result<Item, ClientError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(frame))) => match (this.mapper)(frame) {
                    Ok(Some(item)) => return Poll::Ready(Some(Ok(item))),
                    Ok(None) => {}
                    Err(error) => return Poll::Ready(Some(Err(error))),
                },
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => return Poll::Ready(None),
            }
        }
    }
}
