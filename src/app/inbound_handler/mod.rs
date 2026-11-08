//! Inbound connection handling and route preparation utilities.

use std::{collections::HashMap, sync::Arc};

use log::warn;
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use super::{
    builder::WireframeApp,
    connection_runtime::{ConnectionRuntime, InboundConfig},
    envelope::{Envelope, Packet},
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
    middleware_types::{Handler, Middleware},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    hooks::ProtocolHooks,
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
    pub(crate) serializer: &'a S,
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
    /// Optional byte budgets enforced while processing the connection.
    pub(crate) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(crate) read_timeout_ms: u64,
}

/// Drive a connection using immutable application inputs and prepared routes.
///
/// When a teardown hook is registered and setup produced state, teardown runs
/// exactly once after processing stops: whether stream processing completed
/// cleanly, returned an error such as a malformed frame or a failed send, or
/// panicked inside a handler. A handler panic is finalized and then re-raised.
/// Cooperative cancellation is not requested on this path; see
/// [`process_connection_with_cancellation`].
pub(crate) async fn process_connection<S, C, E, F, W>(
    stream: W,
    context: ConnectionProcessingContext<'_, S, C, E, F>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    process_connection_with_cancellation(stream, context, None).await
}

/// Drive a connection, observing cooperative cancellation at the read
/// boundary.
///
/// The runtime owns the setup state and finalizes it exactly once. Keep the
/// processing result instead of propagating it with `?`: an early return would
/// skip finalization and leak per-connection resources on decode, protocol, or
/// transport errors. A handler panic is finalized and then re-raised so the
/// connection-task supervisor still observes it.
pub(crate) async fn process_connection_with_cancellation<S, C, E, F, W>(
    stream: W,
    context: ConnectionProcessingContext<'_, S, C, E, F>,
    cancellation: Option<CancellationToken>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
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
        memory_budgets,
        read_timeout_ms,
    } = context;

    let mut runtime = ConnectionRuntime::new(
        stream,
        InboundConfig {
            routes,
            serializer,
            codec,
            message_assembler,
            fragmentation,
            protocol_hooks,
            memory_budgets,
            read_timeout_ms,
        },
    );

    // Setup runs before any frame is read. A setup panic or a dropped setup
    // future leaves the finalizer `NotSetUp`, so teardown is never invoked
    // without state.
    if let Some(setup) = on_connect {
        runtime.arm_setup(setup().await);
    }

    let processing_result = runtime.run(cancellation.as_ref(), on_disconnect).await;

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
    handlers: &HashMap<u32, Handler<E>>,
    middleware: &[Box<dyn Middleware<E>>],
) -> HashMap<u32, HandlerService<E>>
where
    E: Packet,
{
    let mut routes = HashMap::new();
    for (&id, handler) in handlers {
        let mut service = HandlerService::new(id, handler.clone());
        for mw in middleware.iter().rev() {
            service = mw.transform(service).await;
        }
        routes.insert(id, service);
    }
    routes
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
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
        process_connection(
            stream,
            ConnectionProcessingContext {
                routes: &routes,
                serializer: &self.serializer,
                codec: &self.codec,
                on_connect: self.on_connect.as_ref(),
                on_disconnect: self.on_disconnect.as_ref(),
                message_assembler: self.message_assembler.as_ref(),
                fragmentation: self.fragmentation,
                protocol_hooks: self.protocol_hooks(),
                memory_budgets: self.memory_budgets,
                read_timeout_ms: self.read_timeout_ms,
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
