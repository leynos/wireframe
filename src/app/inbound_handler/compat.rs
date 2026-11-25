//! Deprecated compatibility entry points on the unprepared application.
//!
//! These methods rebuild a connection's route chains per connection, which
//! defeats preparation. They remain only so callers that predate
//! [`PreparedApp`](crate::app::PreparedApp) keep compiling, and each one
//! forwards to the shared connection path.

use std::{io, sync::Arc};

use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use super::{
    ConnectionProcessingContext,
    Envelope,
    Packet,
    WireframeApp,
    build_route_chains,
    process_connection,
    with_codec_frame_limit,
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Drive one connection through rebuilt routes for compatibility callers.
    ///
    /// Call `app.handle_unprepared_connection_result(stream).await` when a
    /// per-connection factory still supplies a fresh application builder.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    pub(crate) async fn handle_unprepared_connection_result<W>(&self, stream: W) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let routes = build_route_chains(&self.handlers, &self.middleware).await;
        // This compatibility path has no server shutdown signal, so its
        // connection token is never cancelled from outside.
        let shutdown = CancellationToken::new();
        process_connection(
            stream,
            ConnectionProcessingContext {
                routes: &routes,
                streaming_routes: &self.streaming_handlers,
                serializer: self.serializer.as_ref(),
                codec: &self.codec,
                on_connect: self.on_connect.as_deref(),
                on_disconnect: self.on_disconnect.as_deref(),
                message_assembler: self.message_assembler.as_deref(),
                fragmentation: self.fragmentation,
                protocol_hooks: self.protocol_hooks(),
                memory_budgets: self.memory_budgets,
                recovery_hook: self.recovery_hook.as_deref(),
                recovery_config: self.recovery_config.clone(),
                read_timeout_ms: self.read_timeout_ms,
                shutdown: &shutdown,
                push_queues: with_codec_frame_limit(
                    self.push_queues.clone().unwrap_or_default(),
                    &self.codec,
                ),
                session_registry: Arc::clone(&self.session_registry),
                // This compatibility path is handed a stream rather than an
                // accepted socket, so it has no peer address to report.
                peer_addr: None,
            },
        )
        .await
    }

    /// Handle a connection through a compatibility route preparation path.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    #[deprecated(
        since = "0.4.0",
        note = "prepare the app once, then call PreparedApp::handle_connection_result"
    )]
    pub async fn handle_connection_result<W>(&self, stream: W) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.handle_unprepared_connection_result(stream).await
    }

    /// Handle a connection through the compatibility preparation path.
    #[deprecated(
        since = "0.4.0",
        note = "prepare the app once, then call PreparedApp::handle_connection"
    )]
    pub async fn handle_connection<W>(&self, stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Err(error) = self.handle_unprepared_connection_result(stream).await {
            warn!(
                "connection handling completed with error: correlation_id={:?}, error={error:?}",
                None::<u64>
            );
        }
    }
}
