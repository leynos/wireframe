//! Inbound connection handling and route preparation utilities.

mod core;

use std::{collections::HashMap, sync::Arc};

use log::warn;
use tokio::io::{self, AsyncRead, AsyncWrite};

use super::{
    builder::WireframeApp,
    envelope::{Envelope, Packet},
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
    middleware_types::{Handler, Middleware},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
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
    /// Optional byte budgets enforced while processing the connection.
    pub(crate) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(crate) read_timeout_ms: u64,
}

/// Drive a connection using immutable application inputs and prepared routes.
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
    let ConnectionProcessingContext {
        routes,
        serializer,
        codec,
        on_connect,
        on_disconnect,
        message_assembler,
        fragmentation,
        memory_budgets,
        read_timeout_ms,
    } = context;
    let state = if let Some(setup) = on_connect {
        Some(setup().await)
    } else {
        None
    };

    let processing_result = core::process_stream(
        stream,
        core::StreamProcessingContext {
            routes,
            serializer,
            codec,
            message_assembler,
            fragmentation,
            memory_budgets,
            read_timeout_ms,
        },
    )
    .await;

    if let (Some(teardown), Some(state)) = (on_disconnect, state) {
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
    /// Handle a connection through a compatibility route preparation path.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    #[deprecated(note = "prepare the app once, then call PreparedApp::handle_connection_result")]
    pub async fn handle_connection_result<W>(&self, stream: W) -> io::Result<()>
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
                memory_budgets: self.memory_budgets,
                read_timeout_ms: self.read_timeout_ms,
            },
        )
        .await
    }

    /// Handle a connection through the compatibility preparation path.
    #[deprecated(note = "prepare the app once, then call PreparedApp::handle_connection")]
    pub async fn handle_connection<W>(&self, stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        #[expect(
            deprecated,
            reason = "compatibility wrapper delegates to its fallible counterpart"
        )]
        if let Err(error) = self.handle_connection_result(stream).await {
            warn!(
                "connection handling completed with error: correlation_id={:?}, error={error:?}",
                None::<u64>
            );
        }
    }
}

#[cfg(test)]
mod tests;
