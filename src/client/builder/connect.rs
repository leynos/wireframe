//! Connection establishment for `WireframeClientBuilder`.

use std::{net::SocketAddr, sync::atomic::AtomicU64};

use bincode::Encode;
use tokio::net::TcpSocket;
use tokio_util::codec::Framed;

use super::WireframeClientBuilder;
use crate::{
    client::{ClientError, WireframeClient, preamble_exchange::perform_preamble_exchange},
    rewind_stream::RewindStream,
    serializer::Serializer,
};

const INITIAL_READ_BUFFER_CAPACITY: usize = 64 * 1024;

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
    /// use wireframe::client::{ClientError, WireframeClient};
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
        let initial_read_buffer_capacity = core::cmp::min(
            INITIAL_READ_BUFFER_CAPACITY,
            codec_config.max_frame_length_value(),
        );
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
            request_hooks: self.request_hooks,
            correlation_counter: AtomicU64::new(1),
        })
    }
}
