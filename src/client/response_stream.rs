//! Streaming response receiver for consuming multi-frame server responses.
//!
//! [`ResponseStream`] implements [`futures::Stream`] to yield data frames from
//! a `Response::Stream` or `Response::MultiPacket` server response. It
//! validates correlation identifiers on every frame and terminates when the
//! protocol's end-of-stream marker arrives.

use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;

use super::{ClientError, runtime::ClientStream};
use crate::{app::Packet, message::DecodeWith, serializer::Serializer};

/// An async stream of typed frames received from a streaming server response.
///
/// Created by [`WireframeClient::call_streaming`](super::WireframeClient::call_streaming)
/// or [`WireframeClient::receive_streaming`](super::WireframeClient::receive_streaming).
///
/// Each call to [`StreamExt::next`](futures::StreamExt::next) reads the next
/// frame from the transport, deserializes it, validates the correlation
/// identifier, and checks for the protocol's end-of-stream terminator. Data
/// frames are yielded as `Ok(packet)`; the terminator is consumed internally
/// and the stream returns `None`.
///
/// Back-pressure propagates naturally through TCP flow control: if the caller
/// reads slowly, the TCP receive buffer fills, which stalls the server's
/// writes and in turn suspends its response stream or channel.
///
/// # Examples
///
/// ```no_run
/// use std::net::SocketAddr;
///
/// use futures::StreamExt;
/// use wireframe::{
///     app::Envelope,
///     client::{ClientError, WireframeClient},
/// };
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), ClientError> {
/// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
/// let mut client = WireframeClient::builder().connect(addr).await?;
///
/// let request = Envelope::new(1, None, vec![]);
/// let mut stream = client.call_streaming::<Envelope>(request).await?;
///
/// while let Some(result) = stream.next().await {
///     let frame = result?;
///     println!("received frame: {:?}", frame);
/// }
/// // Stream terminated — all frames received.
/// # Ok(())
/// # }
/// ```
pub struct ResponseStream<'a, P, S, T, C>
where
    P: Packet + DecodeWith<S>,
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    client: &'a mut super::WireframeClient<S, T, C>,
    correlation_id: u64,
    terminated: bool,
    frame_count: usize,
    _phantom: PhantomData<fn() -> P>,
}

impl<'a, P, S, T, C> ResponseStream<'a, P, S, T, C>
where
    P: Packet + DecodeWith<S>,
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Create a new streaming response receiver.
    ///
    /// The `correlation_id` is used to validate every inbound frame. Frames
    /// whose correlation identifier does not match are rejected with
    /// [`ClientError::StreamCorrelationMismatch`].
    pub(crate) fn new(
        client: &'a mut super::WireframeClient<S, T, C>,
        correlation_id: u64,
    ) -> Self {
        Self {
            client,
            correlation_id,
            terminated: false,
            frame_count: 0,
            _phantom: PhantomData,
        }
    }

    /// Returns the correlation identifier that frames are validated against.
    #[must_use]
    pub fn correlation_id(&self) -> u64 { self.correlation_id }

    /// Returns `true` if the stream has received the end-of-stream terminator.
    #[must_use]
    pub fn is_terminated(&self) -> bool { self.terminated }

    /// Returns the number of data frames received so far.
    #[must_use]
    pub fn frame_count(&self) -> usize { self.frame_count }

    /// Deserialize raw bytes and validate the resulting packet against the
    /// terminator predicate and expected correlation identifier.
    fn process_frame(&mut self, bytes: &[u8]) -> Option<Result<P, ClientError>> {
        let (packet, _consumed) = match self.client.serializer.deserialize::<P>(bytes) {
            Ok(result) => result,
            Err(e) => {
                self.terminated = true;
                return Some(Err(ClientError::decode(e)));
            }
        };

        // Check terminator before correlation so that end-of-stream frames
        // without a per-request correlation stamp are handled cleanly.
        if packet.is_stream_terminator() {
            self.terminated = true;
            tracing::debug!(
                stream.frames_total = self.frame_count,
                correlation_id = self.correlation_id,
                "stream terminated"
            );
            return None;
        }

        let received_cid = packet.correlation_id();
        if received_cid != Some(self.correlation_id) {
            self.terminated = true;
            return Some(Err(ClientError::StreamCorrelationMismatch {
                expected: Some(self.correlation_id),
                received: received_cid,
            }));
        }

        Some(Ok(packet))
    }
}

impl<P, S, T, C> Stream for ResponseStream<'_, P, S, T, C>
where
    P: Packet + DecodeWith<S>,
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    type Item = Result<P, ClientError>;

    #[expect(
        clippy::cognitive_complexity,
        reason = "linear poll_next with per-frame tracing events"
    )]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.terminated {
            return Poll::Ready(None);
        }

        // Poll the underlying Framed transport directly. Framed<T, Codec> is
        // Unpin when T: Unpin, which ClientStream guarantees.
        match Pin::new(&mut this.client.framed).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                this.terminated = true;
                Poll::Ready(Some(Err(ClientError::disconnected())))
            }
            Poll::Ready(Some(Err(e))) => {
                this.terminated = true;
                Poll::Ready(Some(Err(ClientError::from(e))))
            }
            Poll::Ready(Some(Ok(mut bytes))) => {
                let frame_bytes = bytes.len();
                this.client.invoke_after_receive_hooks(&mut bytes);
                let result = this.process_frame(&bytes);
                if let Some(Ok(_)) = &result {
                    this.frame_count = this.frame_count.saturating_add(1);
                    tracing::debug!(
                        frame.bytes = frame_bytes,
                        stream.frames_received = this.frame_count,
                        correlation_id = this.correlation_id,
                        "stream frame received"
                    );
                }
                Poll::Ready(result)
            }
        }
    }
}
