//! Builder for configuring and connecting a wireframe client.

use std::{net::SocketAddr, time::Duration};

use tokio::net::TcpSocket;
use tokio_util::codec::Framed;

use super::{ClientCodecConfig, ClientError, SocketOptions, WireframeClient};
use crate::{
    frame::LengthFormat,
    serializer::{BincodeSerializer, Serializer},
};

/// Builder for [`WireframeClient`].
///
/// # Examples
///
/// ```
/// use wireframe::client::WireframeClientBuilder;
///
/// let builder = WireframeClientBuilder::new();
/// let _ = builder;
/// ```
pub struct WireframeClientBuilder<S = BincodeSerializer> {
    serializer: S,
    codec_config: ClientCodecConfig,
    socket_options: SocketOptions,
}

impl WireframeClientBuilder<BincodeSerializer> {
    /// Create a new builder with default settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new();
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            serializer: BincodeSerializer,
            codec_config: ClientCodecConfig::default(),
            socket_options: SocketOptions::default(),
        }
    }
}

impl Default for WireframeClientBuilder<BincodeSerializer> {
    fn default() -> Self { Self::new() }
}

impl<S> WireframeClientBuilder<S>
where
    S: Serializer + Send + Sync,
{
    /// Replace the serializer used for encoding and decoding messages.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{BincodeSerializer, client::WireframeClientBuilder};
    ///
    /// let builder = WireframeClientBuilder::new().serializer(BincodeSerializer);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn serializer<Ser>(self, serializer: Ser) -> WireframeClientBuilder<Ser>
    where
        Ser: Serializer + Send + Sync,
    {
        WireframeClientBuilder {
            serializer,
            codec_config: self.codec_config,
            socket_options: self.socket_options,
        }
    }

    /// Configure codec settings for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::{ClientCodecConfig, WireframeClientBuilder};
    ///
    /// let codec = ClientCodecConfig::default().max_frame_length(2048);
    /// let builder = WireframeClientBuilder::new().codec_config(codec);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn codec_config(mut self, codec_config: ClientCodecConfig) -> Self {
        self.codec_config = codec_config;
        self
    }

    /// Configure the maximum frame length for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().max_frame_length(2048);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn max_frame_length(mut self, max_frame_length: usize) -> Self {
        self.codec_config = self.codec_config.max_frame_length(max_frame_length);
        self
    }

    /// Configure the length prefix format for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{client::WireframeClientBuilder, frame::LengthFormat};
    ///
    /// let builder = WireframeClientBuilder::new().length_format(LengthFormat::u16_be());
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn length_format(mut self, length_format: LengthFormat) -> Self {
        self.codec_config = self.codec_config.length_format(length_format);
        self
    }

    /// Replace the socket options applied before connecting.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::{SocketOptions, WireframeClientBuilder};
    ///
    /// let options = SocketOptions::default().nodelay(true);
    /// let builder = WireframeClientBuilder::new().socket_options(options);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn socket_options(mut self, socket_options: SocketOptions) -> Self {
        self.socket_options = socket_options;
        self
    }

    /// Configure `TCP_NODELAY` for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().nodelay(true);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn nodelay(mut self, enabled: bool) -> Self {
        self.socket_options = self.socket_options.nodelay(enabled);
        self
    }

    /// Configure `SO_KEEPALIVE` for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .keepalive(Some(Duration::from_secs(30)));
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn keepalive(mut self, duration: Option<Duration>) -> Self {
        self.socket_options = self.socket_options.keepalive(duration);
        self
    }

    /// Configure TCP linger behaviour for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().linger(Some(Duration::from_secs(1)));
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn linger(mut self, duration: Option<Duration>) -> Self {
        self.socket_options = self.socket_options.linger(duration);
        self
    }

    /// Configure the socket send buffer size.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().send_buffer_size(4096);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn send_buffer_size(mut self, size: u32) -> Self {
        self.socket_options = self.socket_options.send_buffer_size(size);
        self
    }

    /// Configure the socket receive buffer size.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().recv_buffer_size(4096);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn recv_buffer_size(mut self, size: u32) -> Self {
        self.socket_options = self.socket_options.recv_buffer_size(size);
        self
    }

    /// Configure `SO_REUSEADDR` for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().reuseaddr(true);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn reuseaddr(mut self, enabled: bool) -> Self {
        self.socket_options = self.socket_options.reuseaddr(enabled);
        self
    }

    /// Configure `SO_REUSEPORT` for the connection on supported platforms.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().reuseport(true);
    /// let _ = builder;
    /// ```
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    #[must_use]
    pub fn reuseport(mut self, enabled: bool) -> Self {
        self.socket_options = self.socket_options.reuseport(enabled);
        self
    }

    /// Establish a connection and return a configured client.
    ///
    /// # Errors
    /// Returns [`ClientError`] if socket configuration or connection fails.
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
    /// let _client = WireframeClient::builder().connect(addr).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect(self, addr: SocketAddr) -> Result<WireframeClient<S>, ClientError> {
        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        self.socket_options.apply(&socket)?;
        let stream = socket.connect(addr).await?;
        let codec_config = self.codec_config;
        let codec = codec_config.build_codec();
        let mut framed = Framed::new(stream, codec);
        let initial_read_buffer_capacity =
            core::cmp::min(64 * 1024, codec_config.max_frame_length_value());
        framed
            .read_buffer_mut()
            .reserve(initial_read_buffer_capacity);
        Ok(WireframeClient {
            framed,
            serializer: self.serializer,
            codec_config,
        })
    }
}
