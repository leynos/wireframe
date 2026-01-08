//! Builder for configuring and connecting a wireframe client.

use std::{future::Future, net::SocketAddr, sync::Arc, time::Duration};

use bincode::Encode;
use tokio::net::TcpSocket;
use tokio_util::codec::Framed;

use super::{
    ClientCodecConfig,
    ClientError,
    SocketOptions,
    WireframeClient,
    hooks::LifecycleHooks,
    preamble_exchange::{PreambleConfig, perform_preamble_exchange},
};
use crate::{
    frame::LengthFormat,
    rewind_stream::RewindStream,
    serializer::{BincodeSerializer, Serializer},
};

/// Reconstructs `WireframeClientBuilder` with one field updated to a new value.
///
/// This macro reduces duplication in type-changing builder methods that need to
/// create a new builder instance with different generic parameters. When a type
/// parameter changes, struct update syntax (`..self`) cannot be used, so fields
/// must be copied explicitly.
///
/// The `lifecycle_hooks` field requires special handling because `LifecycleHooks<C>`
/// is parameterized by the connection state type. When changing `S` or `P`, the
/// hooks can be moved directly since `C` is unchanged. When changing `C` via
/// `on_connection_setup`, a new `LifecycleHooks<C2>` must be constructed.
macro_rules! builder_field_update {
    // Serializer change: preserves P and C, moves lifecycle_hooks unchanged
    ($self:expr,serializer = $value:expr) => {
        WireframeClientBuilder {
            serializer: $value,
            codec_config: $self.codec_config,
            socket_options: $self.socket_options,
            preamble_config: $self.preamble_config,
            lifecycle_hooks: $self.lifecycle_hooks,
        }
    };
    // Preamble change: preserves S and C, moves lifecycle_hooks unchanged
    ($self:expr,preamble_config = $value:expr) => {
        WireframeClientBuilder {
            serializer: $self.serializer,
            codec_config: $self.codec_config,
            socket_options: $self.socket_options,
            preamble_config: $value,
            lifecycle_hooks: $self.lifecycle_hooks,
        }
    };
    // Lifecycle hooks change: preserves S and P, changes C
    ($self:expr,lifecycle_hooks = $value:expr) => {
        WireframeClientBuilder {
            serializer: $self.serializer,
            codec_config: $self.codec_config,
            socket_options: $self.socket_options,
            preamble_config: $self.preamble_config,
            lifecycle_hooks: $value,
        }
    };
}

/// Builder for [`WireframeClient`].
///
/// The builder supports three generic type parameters:
/// - `S`: The serializer type (default: `BincodeSerializer`)
/// - `P`: The preamble type (default: `()`)
/// - `C`: The connection state type returned by the setup hook (default: `()`)
///
/// # Examples
///
/// ```
/// use wireframe::client::WireframeClientBuilder;
///
/// let builder = WireframeClientBuilder::new();
/// let _ = builder;
/// ```
pub struct WireframeClientBuilder<S = BincodeSerializer, P = (), C = ()> {
    pub(crate) serializer: S,
    pub(crate) codec_config: ClientCodecConfig,
    pub(crate) socket_options: SocketOptions,
    pub(crate) preamble_config: Option<PreambleConfig<P>>,
    pub(crate) lifecycle_hooks: LifecycleHooks<C>,
}

impl WireframeClientBuilder<BincodeSerializer, (), ()> {
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
            lifecycle_hooks: LifecycleHooks::default(),
        }
    }
}

impl Default for WireframeClientBuilder<BincodeSerializer, (), ()> {
    fn default() -> Self { Self::new() }
}

impl<S, P, C> WireframeClientBuilder<S, P, C>
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
    pub fn serializer<Ser>(self, serializer: Ser) -> WireframeClientBuilder<Ser, P, C>
    where
        Ser: Serializer + Send + Sync,
    {
        builder_field_update!(self, serializer = serializer)
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
    pub fn with_preamble<Q>(self, preamble: Q) -> WireframeClientBuilder<S, Q, C>
    where
        Q: Encode + Send + Sync + 'static,
    {
        builder_field_update!(self, preamble_config = Some(PreambleConfig::new(preamble)))
    }

    /// Register a callback invoked when the connection is established.
    ///
    /// The callback can perform authentication or other setup tasks and returns
    /// connection-specific state stored for the connection's lifetime. This
    /// hook is invoked after the preamble exchange (if configured) succeeds.
    ///
    /// # Type Parameters
    ///
    /// This method changes the connection state type parameter from `C` to
    /// `C2`. Subsequent builder methods will operate on the new connection
    /// state type. Note that any previously configured `on_connection_teardown`
    /// hook is cleared because its type signature depends on the old state
    /// type. The `on_error` hook is preserved since it does not depend on
    /// the connection state type.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// struct Session {
    ///     id: u64,
    /// }
    ///
    /// let builder =
    ///     WireframeClientBuilder::new().on_connection_setup(|| async { Session { id: 42 } });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_connection_setup<F, Fut, C2>(self, f: F) -> WireframeClientBuilder<S, P, C2>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = C2> + Send + 'static,
        C2: Send + 'static,
    {
        // Preserve on_error since it is not parameterized by C.
        // on_disconnect must be cleared because its signature depends on C.
        let on_error = self.lifecycle_hooks.on_error;
        builder_field_update!(
            self,
            lifecycle_hooks = LifecycleHooks {
                on_connect: Some(Arc::new(move || Box::pin(f()))),
                on_disconnect: None,
                on_error,
            }
        )
    }
}

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
{
    /// Register a callback invoked when the connection is closed.
    ///
    /// The callback receives the connection state produced by
    /// [`on_connection_setup`](Self::on_connection_setup). The teardown hook
    /// is invoked when [`WireframeClient::close`](super::WireframeClient::close)
    /// is called.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// struct Session {
    ///     id: u64,
    /// }
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .on_connection_setup(|| async { Session { id: 42 } })
    ///     .on_connection_teardown(|session| async move {
    ///         println!("Session {} closed", session.id);
    ///     });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_connection_teardown<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.lifecycle_hooks.on_disconnect = Some(Arc::new(move |c| Box::pin(f(c))));
        self
    }

    /// Register a callback invoked when an error occurs.
    ///
    /// The callback receives a reference to the error and can perform logging
    /// or recovery actions. The handler is invoked before the error is returned
    /// to the caller.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().on_error(|err| async move {
    ///     eprintln!("Client error: {err}");
    /// });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_error<F, Fut>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a ClientError) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.lifecycle_hooks.on_error = Some(Arc::new(move |e| Box::pin(f(e))));
        self
    }
}

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
    P: Encode + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Establish a connection and return a configured client.
    ///
    /// If a preamble is configured, it is written to the server before the
    /// framing layer is established. The success callback (if registered) is
    /// invoked after writing the preamble and may read the server's response.
    /// If a connection setup hook is registered, it is invoked after the
    /// preamble exchange succeeds.
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
    ) -> Result<WireframeClient<S, RewindStream<tokio::net::TcpStream>, C>, ClientError> {
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

        // Invoke connection setup hook if configured.
        let connection_state = if let Some(ref setup) = self.lifecycle_hooks.on_connect {
            Some(setup().await)
        } else {
            None
        };

        Ok(WireframeClient {
            framed,
            serializer: self.serializer,
            codec_config,
            connection_state,
            on_disconnect: self.lifecycle_hooks.on_disconnect,
            on_error: self.lifecycle_hooks.on_error,
        })
    }
}
