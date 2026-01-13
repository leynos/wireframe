//! Wireframe client runtime implementation.

use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::{
    ClientCodecConfig,
    ClientError,
    WireframeClientBuilder,
    hooks::{ClientConnectionTeardownHandler, ClientErrorHandler},
};
use crate::{
    app::Packet,
    message::Message,
    rewind_stream::RewindStream,
    serializer::{BincodeSerializer, Serializer},
};

/// Trait alias for stream types that can be used with the client runtime.
pub trait ClientStream: AsyncRead + AsyncWrite + Unpin {}
impl<T> ClientStream for T where T: AsyncRead + AsyncWrite + Unpin {}

/// Client runtime for wireframe connections.
///
/// The client supports connection lifecycle hooks that mirror the server's
/// hooks, enabling consistent instrumentation across both ends of a wireframe
/// connection.
///
/// # Correlation Identifiers
///
/// When using the envelope-aware APIs ([`send_envelope`](Self::send_envelope),
/// [`receive_envelope`](Self::receive_envelope), and
/// [`call_correlated`](Self::call_correlated)), the client automatically
/// generates unique correlation identifiers for each request. The response
/// is validated to ensure its correlation ID matches the request.
///
/// # Examples
///
/// ```no_run
/// use std::net::SocketAddr;
///
/// use wireframe::WireframeClient;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), wireframe::ClientError> {
/// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
/// let _client = WireframeClient::builder().connect(addr).await?;
/// # Ok(())
/// # }
/// ```
pub struct WireframeClient<S = BincodeSerializer, T = TcpStream, C = ()>
where
    T: ClientStream,
{
    pub(crate) framed: Framed<T, LengthDelimitedCodec>,
    pub(crate) serializer: S,
    pub(crate) codec_config: ClientCodecConfig,
    pub(crate) connection_state: Option<C>,
    pub(crate) on_disconnect: Option<ClientConnectionTeardownHandler<C>>,
    pub(crate) on_error: Option<ClientErrorHandler>,
    /// Counter for generating unique correlation identifiers.
    pub(crate) correlation_counter: AtomicU64,
}

impl<S, T, C> fmt::Debug for WireframeClient<S, T, C>
where
    T: ClientStream,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireframeClient")
            .field("codec_config", &self.codec_config)
            .finish_non_exhaustive()
    }
}

impl WireframeClient<BincodeSerializer, TcpStream, ()> {
    /// Start building a new client with the default serializer and codec.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::WireframeClient;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), wireframe::ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let _client = WireframeClient::builder().connect(addr).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn builder() -> WireframeClientBuilder<BincodeSerializer, (), ()> {
        WireframeClientBuilder::new()
    }
}

impl<S, T, C> WireframeClient<S, T, C>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Send a message to the peer using the configured serializer.
    ///
    /// If an error hook is registered, it is invoked before the error is
    /// returned.
    ///
    /// # Errors
    /// Returns [`ClientError`] if serialization or I/O fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// #[derive(bincode::Encode, bincode::BorrowDecode)]
    /// struct Ping(u8);
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    /// client.send(&Ping(1)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send<M: Message>(&mut self, message: &M) -> Result<(), ClientError> {
        let bytes = match self.serializer.serialize(message) {
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
        Ok(())
    }

    /// Receive the next message from the peer.
    ///
    /// If an error hook is registered, it is invoked before the error is
    /// returned.
    ///
    /// # Errors
    /// Returns [`ClientError`] if the connection closes, decoding fails, or I/O
    /// errors occur.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// #[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
    /// struct Pong(u8);
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    /// let _pong: Pong = client.receive().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive<M: Message>(&mut self) -> Result<M, ClientError> {
        self.receive_internal().await
    }

    /// Send a message and await the next response.
    ///
    /// If an error hook is registered, it is invoked before any error is
    /// returned.
    ///
    /// # Errors
    /// Returns [`ClientError`] if the request cannot be sent or the response
    /// cannot be decoded.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// #[derive(bincode::Encode, bincode::BorrowDecode)]
    /// struct Login {
    ///     username: String,
    /// }
    ///
    /// #[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
    /// struct LoginAck {
    ///     ok: bool,
    /// }
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    /// let login = Login {
    ///     username: "guest".to_string(),
    /// };
    /// let _ack: LoginAck = client.call(&login).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call<Req: Message, Resp: Message>(
        &mut self,
        request: &Req,
    ) -> Result<Resp, ClientError> {
        self.send(request).await?;
        self.receive().await
    }

    /// Inspect the configured codec settings.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let client = WireframeClient::builder().connect(addr).await?;
    /// let codec = client.codec_config();
    /// assert_eq!(codec.max_frame_length_value(), 1024);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn codec_config(&self) -> &ClientCodecConfig { &self.codec_config }

    /// Access the underlying stream.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let client = WireframeClient::builder().connect(addr).await?;
    /// let _stream = client.stream();
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn stream(&self) -> &T { self.framed.get_ref() }

    /// Invoke the error hook if one is registered.
    async fn invoke_error_hook(&self, error: &ClientError) {
        if let Some(ref handler) = self.on_error {
            handler(error).await;
        }
    }

    /// Internal helper for receiving and deserializing a frame.
    async fn receive_internal<R: Message>(&mut self) -> Result<R, ClientError> {
        let Some(frame) = self.framed.next().await else {
            let err = ClientError::Disconnected;
            self.invoke_error_hook(&err).await;
            return Err(err);
        };
        let bytes = match frame {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ClientError::from(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };
        let (message, _consumed) = match self.serializer.deserialize(&bytes) {
            Ok(result) => result,
            Err(e) => {
                let err = ClientError::Deserialize(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };
        Ok(message)
    }

    /// Generate the next unique correlation identifier for this connection.
    ///
    /// Correlation identifiers are generated sequentially starting from 1.
    /// Each call returns a unique value within this client instance.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let client = WireframeClient::builder().connect(addr).await?;
    /// let id1 = client.next_correlation_id();
    /// let id2 = client.next_correlation_id();
    /// assert_ne!(id1, id2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn next_correlation_id(&self) -> u64 {
        self.correlation_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Send an envelope to the peer using the configured serializer.
    ///
    /// If the envelope does not have a correlation ID set, one is automatically
    /// generated and stamped on the envelope before sending.
    ///
    /// Returns the correlation ID that was used (either the existing one or
    /// the auto-generated one).
    ///
    /// If an error hook is registered, it is invoked before the error is
    /// returned.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if serialization or I/O fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    ///
    /// // Correlation ID will be auto-generated
    /// let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    /// let correlation_id = client.send_envelope(envelope).await?;
    /// assert!(correlation_id > 0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_envelope<P: Packet>(&mut self, envelope: P) -> Result<u64, ClientError> {
        // Check once whether correlation ID is present.
        let existing = envelope.correlation_id();
        let correlation_id = existing.unwrap_or_else(|| self.next_correlation_id());

        // Rebuild envelope with correlation ID if it was auto-generated.
        let envelope = if existing.is_none() {
            let parts = envelope.into_parts();
            P::from_parts(crate::app::PacketParts::new(
                parts.id(),
                Some(correlation_id),
                parts.payload(),
            ))
        } else {
            envelope
        };

        let bytes = match self.serializer.serialize(&envelope) {
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
        Ok(correlation_id)
    }

    /// Receive the next envelope from the peer.
    ///
    /// If an error hook is registered, it is invoked before the error is
    /// returned.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the connection closes, decoding fails, or I/O
    /// errors occur.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    /// let envelope: Envelope = client.receive_envelope().await?;
    /// let _correlation_id = envelope.correlation_id();
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive_envelope<P: Packet>(&mut self) -> Result<P, ClientError> {
        self.receive_internal().await
    }

    /// Send an envelope and await a correlated response.
    ///
    /// This method auto-generates a correlation ID for the request (if not
    /// present), sends the envelope, receives the response, and validates that
    /// the response's correlation ID matches the request.
    ///
    /// If an error hook is registered, it is invoked before any error is
    /// returned.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::CorrelationMismatch`] if the response correlation
    /// ID does not match the request. Returns other [`ClientError`] variants
    /// for serialization, deserialization, or I/O failures.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient, app::Envelope};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    ///
    /// let request = Envelope::new(1, None, vec![1, 2, 3]);
    /// let response: Envelope = client.call_correlated(request).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call_correlated<P: Packet>(&mut self, request: P) -> Result<P, ClientError> {
        let correlation_id = self.send_envelope(request).await?;
        let response: P = self.receive_envelope().await?;

        // Validate correlation ID matches.
        let response_correlation_id = response.correlation_id();
        if response_correlation_id != Some(correlation_id) {
            let err = ClientError::CorrelationMismatch {
                expected: Some(correlation_id),
                received: response_correlation_id,
            };
            self.invoke_error_hook(&err).await;
            return Err(err);
        }

        Ok(response)
    }
}

impl<S, C> WireframeClient<S, RewindStream<TcpStream>, C>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
{
    /// Access the underlying [`TcpStream`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let client = WireframeClient::builder().connect(addr).await?;
    /// let _stream = client.tcp_stream();
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn tcp_stream(&self) -> &TcpStream { self.framed.get_ref().inner() }

    /// Access the rewind stream wrapper.
    ///
    /// This provides access to the [`RewindStream`] that wraps the TCP stream,
    /// which may contain leftover bytes from preamble exchange.
    #[must_use]
    pub fn rewind_stream(&self) -> &RewindStream<TcpStream> { self.framed.get_ref() }

    /// Gracefully close the connection, invoking teardown hooks.
    ///
    /// This method flushes any pending frames and sends EOF to the peer before
    /// invoking the teardown hook. If a teardown hook was registered via
    /// [`on_connection_teardown`](super::WireframeClientBuilder::on_connection_teardown),
    /// it is invoked with the connection state produced by the setup hook.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{ClientError, WireframeClient};
    ///
    /// struct Session {
    ///     id: u64,
    /// }
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let client = WireframeClient::builder()
    ///     .on_connection_setup(|| async { Session { id: 42 } })
    ///     .on_connection_teardown(|session| async move {
    ///         println!("Session {} closed", session.id);
    ///     })
    ///     .connect(addr)
    ///     .await?;
    /// client.close().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn close(mut self) {
        // Flush pending frames and send EOF before teardown.
        // Ignore errors since we're closing anyway.
        let _ = self.framed.close().await;

        if let (Some(state), Some(handler)) = (self.connection_state.take(), &self.on_disconnect) {
            handler(state).await;
        }
    }
}
