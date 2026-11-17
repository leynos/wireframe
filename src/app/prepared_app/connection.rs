//! Connection handling for a prepared application.
//!
//! Kept separate from the preparation and accessor surface so each file stays
//! small; every method here drives one accepted stream through the prepared
//! routes.

use std::{net::SocketAddr, sync::Arc};

use monotony::{MonotonicClock, StdMonotonicClock};
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::PreparedApp;
use crate::{
    app::{
        envelope::Packet,
        inbound_handler::{ConnectionProcessingContext, process_connection},
    },
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    metrics,
    serializer::Serializer,
};

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = crate::app::Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    crate::app::Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Handle an accepted connection using the prepared route services.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// prepared.handle_connection_result(server).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    pub async fn handle_connection_result<W>(&self, stream: W) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.handle_connection_from(stream, None).await
    }

    /// Handle an accepted connection whose peer address is known.
    ///
    /// The address reaches the recovery hook through
    /// [`CodecErrorContext::peer_address`](crate::codec::CodecErrorContext::peer_address),
    /// so a hook can attribute a decoder failure to the peer that caused it.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::SocketAddr;
    ///
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// let peer: SocketAddr = "127.0.0.1:5000".parse()?;
    /// prepared.handle_connection_from(server, Some(peer)).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    pub async fn handle_connection_from<W>(
        &self,
        stream: W,
        peer_address: Option<SocketAddr>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.handle_connection_with_shutdown(stream, peer_address, CancellationToken::new())
            .await
    }

    /// Handle an accepted connection, stopping it when `shutdown` is cancelled.
    ///
    /// The server passes its worker shutdown token here so a connection whose
    /// peer stopped reading cannot pin a graceful drain: its pending write is
    /// abandoned, which closes the connection. Callers outside the server keep
    /// using [`Self::handle_connection_result`], whose token is never
    /// cancelled.
    pub(crate) async fn handle_connection_with_shutdown<W>(
        &self,
        stream: W,
        peer_address: Option<SocketAddr>,
        shutdown: CancellationToken,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.handle_connection_inner(stream, peer_address, &StdMonotonicClock, &shutdown)
            .await
    }

    /// Handle a connection with an injectable clock and shutdown signal.
    pub(super) async fn handle_connection_inner<W>(
        &self,
        stream: W,
        peer_address: Option<SocketAddr>,
        clock: &dyn MonotonicClock,
        shutdown: &CancellationToken,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        metrics::inc_prepared_connection_uses();
        let started_at = clock.now();
        let span = tracing::info_span!(
            "prepared_connection",
            outcome = tracing::field::Empty,
            elapsed_ms = tracing::field::Empty
        );
        let result = process_connection(
            stream,
            ConnectionProcessingContext {
                routes: &self.routes,
                serializer: Arc::clone(&self.serializer),
                codec: &self.codec,
                on_connect: self.on_connect.as_ref(),
                on_disconnect: self.on_disconnect.as_ref(),
                message_assembler: self.message_assembler.as_ref(),
                fragmentation: self.fragmentation,
                protocol_hooks: self.protocol_hooks(),
                protocol: self.protocol.clone(),
                memory_budgets: self.memory_budgets,
                read_timeout_ms: self.read_timeout_ms,
                shutdown,
                recovery_hook: self.recovery_hook.as_ref(),
                recovery_config: self.recovery_config.clone(),
                peer_address,
            },
        )
        .instrument(span.clone())
        .await;
        span.record("outcome", if result.is_ok() { "success" } else { "error" });
        let elapsed_ms =
            u64::try_from(clock.now().duration_since(started_at).as_millis()).unwrap_or(u64::MAX);
        span.record("elapsed_ms", elapsed_ms);
        result
    }

    /// Handle an accepted connection and log any processing failure.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// prepared.handle_connection(server).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle_connection<W>(&self, stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Err(error) = self.handle_connection_result(stream).await {
            log::warn!(
                "connection handling completed with error: correlation_id={:?}, error={error:?}",
                None::<u64>
            );
        }
    }
}
