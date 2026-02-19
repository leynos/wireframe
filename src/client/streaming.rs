//! Streaming response methods for the wireframe client.
//!
//! These methods enable consumption of `Response::Stream` and
//! `Response::MultiPacket` server responses by returning a
//! [`ResponseStream`](super::ResponseStream) that yields data frames until the
//! protocol's end-of-stream terminator arrives.

use std::sync::atomic::Ordering;

use bytes::Bytes;
use futures::SinkExt;

use super::{ClientError, ResponseStream, runtime::ClientStream};
use crate::{app::Packet, serializer::Serializer};

impl<S, T, C> super::WireframeClient<S, T, C>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Send a request and return a stream of correlated response frames.
    ///
    /// This is the high-level streaming API. It auto-generates a correlation
    /// identifier for the request (if one is not already set), sends the
    /// request, and returns a [`ResponseStream`] that yields data frames
    /// until the server's end-of-stream terminator arrives.
    ///
    /// The terminator frame (identified by
    /// [`Packet::is_stream_terminator`]) is consumed internally and is not
    /// yielded to the caller. The stream returns `None` once the terminator
    /// is received.
    ///
    /// Back-pressure propagates naturally through TCP flow control: if the
    /// caller reads slowly, the server's writes stall.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the request cannot be serialized or the
    /// transport write fails. Per-frame errors (decode failures, correlation
    /// mismatches, disconnects) are surfaced through the stream's
    /// `Item = Result<P, ClientError>`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use futures::StreamExt;
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
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
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call_streaming<P: Packet>(
        &mut self,
        mut request: P,
    ) -> Result<ResponseStream<'_, P, S, T, C>, ClientError> {
        let existing = request.correlation_id();
        let correlation_id =
            existing.unwrap_or_else(|| self.correlation_counter.fetch_add(1, Ordering::Relaxed));

        if existing.is_none() {
            request.set_correlation_id(Some(correlation_id));
        }

        let bytes = match self.serializer.serialize(&request) {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ClientError::Serialize(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };
        if let Err(e) = self.framed.send(Bytes::from(bytes)).await {
            let err = ClientError::from(e);
            self.invoke_error_hook(&err).await;
            return Err(err);
        }

        Ok(ResponseStream::new(self, correlation_id))
    }

    /// Return a stream of correlated response frames for a previously sent
    /// request.
    ///
    /// This is the lower-level streaming API for cases where the caller has
    /// already sent the request via [`send_envelope`](Self::send_envelope)
    /// or [`send`](Self::send) and knows the correlation identifier.
    ///
    /// The returned [`ResponseStream`] validates that every inbound frame
    /// carries the given `correlation_id` and terminates when a frame
    /// satisfying [`Packet::is_stream_terminator`] arrives.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use futures::StreamExt;
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    ///
    /// let request = Envelope::new(1, None, vec![]);
    /// let correlation_id = client.send_envelope(request).await?;
    /// let mut stream = client.receive_streaming::<Envelope>(correlation_id);
    ///
    /// while let Some(result) = stream.next().await {
    ///     let frame = result?;
    ///     println!("received frame: {:?}", frame);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn receive_streaming<P: Packet>(
        &mut self,
        correlation_id: u64,
    ) -> ResponseStream<'_, P, S, T, C> {
        ResponseStream::new(self, correlation_id)
    }
}
