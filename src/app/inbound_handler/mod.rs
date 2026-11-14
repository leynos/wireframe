//! Inbound connection handling and route preparation utilities.

mod actor;
mod core;
mod frames;
mod framing;
mod peer_watch;

use std::{collections::HashMap, sync::Arc};

use log::warn;
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use super::{
    Route,
    builder::WireframeApp,
    envelope::{Envelope, Packet},
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
    middleware_types::Middleware,
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    hooks::{ProtocolHooks, WireframeProtocol},
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    serializer::Serializer,
};

/// Maximum consecutive deserialization failures before closing a connection.
pub(super) const MAX_DESER_FAILURES: u32 = 10;

/// Immutable inputs required to drive one connection through prepared routes.
pub(crate) struct ConnectionProcessingContext<'a, S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    pub(crate) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Serializer shared by all frames on the connection.
    pub(crate) serializer: Arc<S>,
    /// Codec configuration shared by all frames on the connection.
    pub(crate) codec: &'a F,
    /// Optional hook that creates per-connection state.
    pub(crate) on_connect: Option<&'a Arc<ConnectionSetup<C>>>,
    /// Optional hook that releases per-connection state after processing.
    pub(crate) on_disconnect: Option<&'a Arc<ConnectionTeardown<C>>>,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(crate) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Fragmentation settings used to initialize the frame pipeline.
    pub(crate) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Protocol hooks built for this connection from the installed protocol.
    pub(crate) protocol_hooks: ProtocolHooks<F::Frame, ()>,
    /// Protocol whose response hooks drive the connection actor.
    pub(crate) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional byte budgets enforced while processing the connection.
    pub(crate) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(crate) read_timeout_ms: u64,
    /// Server shutdown signal; cancelling it stops this connection.
    pub(crate) shutdown: &'a CancellationToken,
}

/// Drive a connection using immutable application inputs and prepared routes.
///
/// When a teardown hook is registered and setup produced state, teardown runs
/// exactly once before this function returns, whether stream processing
/// completed cleanly or returned an error such as a malformed frame or a
/// failed send. Panic and cancellation finalization are outside this ordering
/// and remain tracked by ADR-012 and issue #643.
pub(crate) async fn process_connection<S, C, E, F, W>(
    stream: W,
    context: ConnectionProcessingContext<'_, S, C, E, F>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let ConnectionProcessingContext {
        routes,
        serializer,
        codec,
        on_connect,
        on_disconnect,
        message_assembler,
        fragmentation,
        protocol_hooks,
        protocol,
        memory_budgets,
        read_timeout_ms,
        shutdown,
    } = context;
    let connection_state = if let Some(setup) = on_connect {
        Some(setup().await)
    } else {
        None
    };

    // Keep the processing result instead of propagating it with `?`. When
    // setup has produced state and a teardown hook is registered, an early
    // return here would skip teardown and leak per-connection resources on
    // decode, protocol, or transport errors. Do not add `?` to this await:
    // teardown must observe every path that can still return a value. Panic
    // and cancellation are not finalized here; see ADR-012 and issue #643.
    let processing_result = core::process_stream(
        stream,
        core::StreamProcessingContext {
            routes,
            serializer,
            codec,
            message_assembler,
            fragmentation,
            protocol_hooks,
            protocol,
            memory_budgets,
            read_timeout_ms,
            shutdown,
        },
    )
    .await;

    if let (Some(teardown), Some(state)) = (on_disconnect, connection_state) {
        teardown(state).await;
    }

    if let Err(error) = processing_result {
        warn!(
            "connection terminated with error: correlation_id={:?}, error={error:?}",
            None::<u64>
        );
        return Err(error);
    }

    Ok(())
}

/// Construct each route's middleware chain from registered builder inputs.
///
/// Middleware is folded in reverse registration order, preserving the first
/// registered middleware as the outermost service layer.
pub(crate) async fn build_route_chains<E>(
    handlers: &HashMap<u32, Route<E>>,
    middleware: &[Box<dyn Middleware<E>>],
) -> HashMap<u32, HandlerService<E>>
where
    E: Packet,
{
    let mut routes = HashMap::new();
    for (&id, route) in handlers {
        let mut service = HandlerService::from_route(id, route);
        for mw in middleware.iter().rev() {
            service = mw.transform(service).await;
        }
        routes.insert(id, service);
    }
    routes
}

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
                shutdown: &shutdown,
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

#[cfg(test)]
mod tests;
