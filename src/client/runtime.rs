//! Wireframe client runtime implementation.

use std::fmt;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::{ClientCodecConfig, ClientError, WireframeClientBuilder};
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
pub struct WireframeClient<S = BincodeSerializer, T = TcpStream>
where
    T: ClientStream,
{
    pub(crate) framed: Framed<T, LengthDelimitedCodec>,
    pub(crate) serializer: S,
    pub(crate) codec_config: ClientCodecConfig,
}

impl<S, T> fmt::Debug for WireframeClient<S, T>
where
    T: ClientStream,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireframeClient")
            .field("codec_config", &self.codec_config)
            .finish_non_exhaustive()
    }
}

impl WireframeClient<BincodeSerializer, TcpStream> {
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
    pub fn builder() -> WireframeClientBuilder<BincodeSerializer, ()> {
        WireframeClientBuilder::new()
    }
}

impl<S, T> WireframeClient<S, T>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Send a message to the peer using the configured serializer.
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
        let bytes = self
            .serializer
            .serialize(message)
            .map_err(ClientError::Serialize)?;
        self.framed.send(Bytes::from(bytes)).await?;
        Ok(())
    }

    /// Receive the next message from the peer.
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
            return Err(ClientError::Disconnected);
        };
        let bytes = frame?;
        let (message, _consumed) = self
            .serializer
            .deserialize(&bytes)
            .map_err(ClientError::Deserialize)?;
        Ok(message)
    }

    /// Send a message and await the next response.
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
}

impl<S> WireframeClient<S, RewindStream<TcpStream>>
where
    S: Serializer + Send + Sync,
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
}
