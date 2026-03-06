//! Connection establishment for `WireframeClientBuilder`.

use std::{net::SocketAddr, time::Instant};

use bincode::Encode;
use tracing::Instrument;

use super::WireframeClientBuilder;
use crate::{
    client::{
        ClientError,
        WireframeClient,
        connect_parts::ClientBuildParts,
        tracing_helpers::{connect_span, emit_timing_event},
    },
    rewind_stream::RewindStream,
    serializer::Serializer,
};

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
        let span = connect_span(&self.tracing_config, &addr.to_string());
        let timing_start = self.tracing_config.connect_timing.then(Instant::now);

        async {
            let result = self.connect_inner(addr).await;
            emit_timing_event(timing_start);
            result
        }
        .instrument(span)
        .await
    }

    /// Perform socket creation, connection, preamble exchange, and codec setup.
    async fn connect_inner(
        self,
        addr: SocketAddr,
    ) -> Result<WireframeClient<S, RewindStream<tokio::net::TcpStream>, C>, ClientError> {
        ClientBuildParts {
            serializer: self.serializer,
            codec_config: self.codec_config,
            socket_options: self.socket_options,
            preamble_config: self.preamble_config,
            lifecycle_hooks: self.lifecycle_hooks,
            request_hooks: self.request_hooks,
            tracing_config: self.tracing_config,
        }
        .connect(addr)
        .await
    }
}
