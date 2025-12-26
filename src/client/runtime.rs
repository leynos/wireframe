//! Wireframe client runtime implementation.

use std::fmt;

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
        let bytes = self.serializer.serialize(message).map_err(|e| {
            let err = ClientError::Serialize(e);
            self.invoke_error_hook(&err);
            err
        })?;
        self.framed.send(Bytes::from(bytes)).await.map_err(|e| {
            let err = ClientError::from(e);
            self.invoke_error_hook(&err);
            err
        })
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
        let Some(frame) = self.framed.next().await else {
            let err = ClientError::Disconnected;
            self.invoke_error_hook(&err);
            return Err(err);
        };
        let bytes = frame.map_err(|e| {
            let err = ClientError::from(e);
            self.invoke_error_hook(&err);
            err
        })?;
        let (message, _consumed) = self.serializer.deserialize(&bytes).map_err(|e| {
            let err = ClientError::Deserialize(e);
            self.invoke_error_hook(&err);
            err
        })?;
        Ok(message)
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
    fn invoke_error_hook(&self, error: &ClientError) {
        if let Some(ref handler) = self.on_error {
            // Execute the error hook synchronously in a blocking context.
            // We use futures::executor::block_on because the error hook is
            // designed for logging/metrics, not for async recovery.
            futures::executor::block_on(handler(error));
        }
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
    /// If a teardown hook was registered via
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
        if let (Some(state), Some(handler)) = (self.connection_state.take(), &self.on_disconnect) {
            handler(state).await;
        }
    }
}
