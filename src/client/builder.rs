//! Builder for configuring and connecting a wireframe client.

use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use bincode::Encode;
use futures::future::BoxFuture;
use log::error;
use tokio::{net::TcpSocket, time::timeout};
use tokio_util::codec::Framed;

use super::{
    ClientCodecConfig,
    ClientError,
    ClientPreambleFailureHandler,
    ClientPreambleSuccessHandler,
    SocketOptions,
    WireframeClient,
};
use crate::{
    frame::LengthFormat,
    preamble::write_preamble,
    rewind_stream::RewindStream,
    serializer::{BincodeSerializer, Serializer},
};

/// Holds optional preamble configuration for the client builder.
struct PreambleConfig<P> {
    preamble: P,
    on_success: Option<ClientPreambleSuccessHandler<P>>,
    on_failure: Option<ClientPreambleFailureHandler>,
    timeout: Option<Duration>,
}

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
pub struct WireframeClientBuilder<S = BincodeSerializer, P = ()> {
    serializer: S,
    codec_config: ClientCodecConfig,
    socket_options: SocketOptions,
    preamble_config: Option<PreambleConfig<P>>,
}

impl WireframeClientBuilder<BincodeSerializer, ()> {
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
            preamble_config: None,
        }
    }
}

impl Default for WireframeClientBuilder<BincodeSerializer, ()> {
    fn default() -> Self { Self::new() }
}

impl<S, P> WireframeClientBuilder<S, P>
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
    pub fn serializer<Ser>(self, serializer: Ser) -> WireframeClientBuilder<Ser, P>
    where
        Ser: Serializer + Send + Sync,
    {
        WireframeClientBuilder {
            serializer,
            codec_config: self.codec_config,
            socket_options: self.socket_options,
            preamble_config: self.preamble_config,
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
    /// let builder = WireframeClientBuilder::new().keepalive(Some(Duration::from_secs(30)));
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

    /// Configure a preamble to send before exchanging frames.
    ///
    /// The preamble is written to the server immediately after establishing
    /// the TCP connection, before the framing layer begins. Use
    /// [`on_preamble_success`](Self::on_preamble_success) to read the server's
    /// response and [`preamble_timeout`](Self::preamble_timeout) to bound the
    /// exchange.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble {
    ///     version: u16,
    /// }
    ///
    /// let builder = WireframeClientBuilder::new().with_preamble(MyPreamble { version: 1 });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn with_preamble<Q>(self, preamble: Q) -> WireframeClientBuilder<S, Q>
    where
        Q: Encode + Send + Sync + 'static,
    {
        WireframeClientBuilder {
            serializer: self.serializer,
            codec_config: self.codec_config,
            socket_options: self.socket_options,
            preamble_config: Some(PreambleConfig {
                preamble,
                on_success: None,
                on_failure: None,
                timeout: None,
            }),
        }
    }
}

impl<S, P> WireframeClientBuilder<S, P>
where
    S: Serializer + Send + Sync,
    P: Encode + Send + Sync + 'static,
{
    /// Configure a timeout for the preamble exchange.
    ///
    /// The timeout is applied to the entire preamble phase: writing the
    /// preamble and running the success callback (which may read the server's
    /// response). Values below 1 ms are clamped to 1 ms to avoid immediate
    /// expiry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble(u8);
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .with_preamble(MyPreamble(1))
    ///     .preamble_timeout(Duration::from_secs(1));
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn preamble_timeout(mut self, timeout: Duration) -> Self {
        let normalised = timeout.max(Duration::from_millis(1));
        if let Some(ref mut config) = self.preamble_config {
            config.timeout = Some(normalised);
        }
        self
    }

    /// Register a handler invoked after the preamble is successfully written.
    ///
    /// The handler receives the sent preamble and a mutable reference to the
    /// TCP stream. It may read the server's response preamble from the stream.
    /// Any leftover bytes (read beyond the server's response) must be returned
    /// so they can be replayed before framed communication begins.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::FutureExt;
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble(u8);
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .with_preamble(MyPreamble(1))
    ///     .on_preamble_success(|_preamble, _stream| {
    ///         async move {
    ///             // Read server response if needed...
    ///             Ok(Vec::new()) // No leftover bytes
    ///         }
    ///         .boxed()
    ///     });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_preamble_success<H>(mut self, handler: H) -> Self
    where
        H: for<'a> Fn(&'a P, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<Vec<u8>>>
            + Send
            + Sync
            + 'static,
    {
        if let Some(ref mut config) = self.preamble_config {
            config.on_success = Some(Arc::new(handler));
        }
        self
    }

    /// Register a handler invoked when the preamble exchange fails.
    ///
    /// The handler receives the error and a mutable reference to the TCP
    /// stream, allowing it to log or send an error response before the
    /// connection closes.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::FutureExt;
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble(u8);
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .with_preamble(MyPreamble(1))
    ///     .on_preamble_failure(|err, _stream| {
    ///         async move {
    ///             eprintln!("Preamble failed: {err}");
    ///             Ok(())
    ///         }
    ///         .boxed()
    ///     });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_preamble_failure<H>(mut self, handler: H) -> Self
    where
        H: for<'a> Fn(
                &'a ClientError,
                &'a mut tokio::net::TcpStream,
            ) -> BoxFuture<'a, io::Result<()>>
            + Send
            + Sync
            + 'static,
    {
        if let Some(ref mut config) = self.preamble_config {
            config.on_failure = Some(Arc::new(handler));
        }
        self
    }

    /// Establish a connection and return a configured client.
    ///
    /// If a preamble is configured, it is written to the server before the
    /// framing layer is established. The success callback (if registered) is
    /// invoked after writing the preamble and may read the server's response.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if socket configuration, connection, or
    /// preamble exchange fails.
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
    pub async fn connect(
        self,
        addr: SocketAddr,
    ) -> Result<WireframeClient<S, RewindStream<tokio::net::TcpStream>>, ClientError> {
        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        self.socket_options.apply(&socket)?;
        let mut stream = socket.connect(addr).await?;

        // Perform preamble exchange if configured.
        let leftover = if let Some(config) = self.preamble_config {
            perform_preamble_exchange(&mut stream, config).await?
        } else {
            Vec::new()
        };

        // Build framed codec, always wrapping in RewindStream for type consistency.
        // When leftover is empty, RewindStream has negligible overhead.
        let codec_config = self.codec_config;
        let codec = codec_config.build_codec();
        let mut framed = Framed::new(RewindStream::new(leftover, stream), codec);
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

/// Perform the preamble exchange: write preamble, invoke success callback.
async fn perform_preamble_exchange<P>(
    stream: &mut tokio::net::TcpStream,
    config: PreambleConfig<P>,
) -> Result<Vec<u8>, ClientError>
where
    P: Encode + Send + Sync + 'static,
{
    let PreambleConfig {
        preamble,
        on_success,
        on_failure,
        timeout: preamble_timeout,
    } = config;

    let result = run_preamble_exchange(stream, &preamble, on_success, preamble_timeout).await;

    // On failure, invoke the failure callback if registered.
    if let Err(ref err) = result {
        invoke_failure_handler(stream, err, on_failure.as_ref()).await;
    }

    result
}

/// Execute the preamble write and success callback, with optional timeout.
async fn run_preamble_exchange<P>(
    stream: &mut tokio::net::TcpStream,
    preamble: &P,
    on_success: Option<ClientPreambleSuccessHandler<P>>,
    preamble_timeout: Option<std::time::Duration>,
) -> Result<Vec<u8>, ClientError>
where
    P: Encode + Send + Sync + 'static,
{
    let exchange = async {
        // Write the preamble.
        write_preamble(stream, preamble)
            .await
            .map_err(ClientError::PreambleEncode)?;

        // Invoke success callback if registered, collecting leftover bytes.
        match on_success.as_ref() {
            Some(handler) => handler(preamble, stream)
                .await
                .map_err(ClientError::PreambleWrite),
            None => Ok(Vec::new()),
        }
    };

    // Apply timeout if configured.
    match preamble_timeout {
        Some(limit) => timeout(limit, exchange)
            .await
            .unwrap_or(Err(ClientError::PreambleTimeout)),
        None => exchange.await,
    }
}

/// Invoke the failure handler if one is registered.
async fn invoke_failure_handler(
    stream: &mut tokio::net::TcpStream,
    err: &ClientError,
    on_failure: Option<&ClientPreambleFailureHandler>,
) {
    if let Some(handler) = on_failure
        && let Err(e) = handler(err, stream).await
    {
        error!("preamble failure handler error: {e}");
    }
}
