//! Inbound connection handling and route preparation utilities.
//!
//! Everything between an accepted socket and a dispatched envelope lives here.
//! A prepared route calls [`process_connection`] with a
//! [`ConnectionProcessingContext`], which carries the immutable application
//! inputs for one connection: the middleware chains, serializer, codec,
//! assembly and fragmentation settings, protocol hooks, read timeout, the
//! server stop token, the push queue template, the [`SessionRegistry`], and the
//! peer address. That function owns setup and teardown around the framed work,
//! which it hands to `core`.
//!
//! [`process_connection`] and its inputs are split across the submodules by
//! responsibility:
//!
//! - `core` drives one connection: it prepares framing, hands the response actor its connection
//!   context, and then waits for pushes or inbound frames on each pass.
//! - `actor` builds the response actor: `build_connection_actor` allocates the connection's
//!   identifier and publishes its push handle in one step, and installs the guard that deregisters
//!   that entry when the connection ends.
//! - `frames` holds the buffered inbound frame and the read deadline, and runs the timeout's expiry
//!   command.
//! - `push_service` races queued pushes against the reader, writes the batch the actor hands over,
//!   and drains the lanes once the peer closes.
//! - `push_write` bounds each push write by the stop token and, when the peer can no longer be
//!   observed, by the idle grace.
//! - `dispatch` decodes an envelope and runs it through the middleware chain.
//! - `framing` and `peer_watch` prepare the transport halves and observe the peer while a response
//!   is written.
//! - `tests` covers envelope decoding, decode-failure diagnostics, and the unprepared compatibility
//!   path.
//!
//! Route preparation lives here too. The server spawner forwards the accepted
//! socket's peer address through `PreparedApp::handle_connection_with_shutdown`,
//! so the TCP path reaches [`process_connection`] with the peer the preamble
//! hooks were given. The `WireframeApp` compatibility path calls
//! [`process_connection`] directly from `handle_unprepared_connection_result`,
//! which is handed a stream rather than an accepted socket, so it passes `None`
//! and its diagnostics name no peer. Either way the connection's push handle is
//! registered before the first frame is read and removed again when the loop
//! ends.

mod actor;
// The connection-local runtime drives the same frame loop from outside this
// module, so the processing entry point is crate-visible.
pub(crate) mod core;
pub(crate) mod dispatch;
mod frames;
mod framing;
mod peer_watch;
mod push_service;
mod push_write;

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use log::warn;
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use super::{
    Route,
    builder::{WireframeApp, push::with_codec_frame_limit},
    connection_runtime::{ConnectionRuntime, InboundConfig},
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
    push::PushQueuesBuilder,
    serializer::Serializer,
    session::SessionRegistry,
};

/// Benchmark entry point for the inbound decode seam.
///
/// Mirrors the crate-internal envelope decoder for harnesses that live outside
/// the crate. It exists only with the `test-support` feature and returns the
/// decoded envelope rather than any internal state, so a caller cannot depend
/// on how the decode is staged.
///
/// # Errors
///
/// Returns the decoder's error when `payload` is not a complete envelope, or
/// when trailing bytes follow it.
///
/// # Examples
///
/// ```ignore
/// let (envelope, consumed) = benchmark_decode_envelope(&BincodeSerializer, &frame_payload)?;
/// assert_eq!(envelope.payload_bytes().len(), payload_len);
/// ```
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn benchmark_decode_envelope<S>(
    serializer: &S,
    payload: &[u8],
) -> std::result::Result<(Envelope, usize), Box<dyn std::error::Error + Send + Sync>>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    Envelope: DecodeWith<S>,
{
    dispatch::parse_envelope(serializer, payload)
}

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
    /// Template cloned to build each connection's push queues.
    pub(crate) push_queues: PushQueuesBuilder<F::Frame>,
    /// Registry that each connection registers its push handle with.
    pub(crate) session_registry: Arc<SessionRegistry<F::Frame>>,
    /// Peer address of the accepted socket, when the server reported one.
    pub(crate) peer_addr: Option<SocketAddr>,
}

/// Drive a connection using immutable application inputs and prepared routes.
///
/// One [`ConnectionRuntime`] owns the per-connection state for the whole
/// call. When a teardown hook is registered and setup produced state, teardown
/// runs exactly once after processing stops: whether stream processing
/// completed cleanly, returned an error such as a malformed frame or a failed
/// send, observed shutdown, or panicked inside a handler. A handler panic is
/// finalized and then re-raised so the connection-task supervisor still counts
/// and logs it.
///
/// Setup completes before any framing state is built, and the framed transport
/// is released before teardown is awaited.
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
        push_queues,
        session_registry,
        peer_addr,
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
            protocol,
            memory_budgets,
            read_timeout_ms,
            shutdown,
            push_queues,
            session_registry,
            peer_addr,
        },
    );

    // Setup runs before any frame is read. A setup panic or a dropped setup
    // future leaves the finalizer `NotSetUp`, so teardown is never invoked
    // without state. `ConnectionRuntime::new` only stages the transport: the
    // codec clone and frame-buffer reservation happen inside `run`, after this
    // await, so a pending setup cannot retain connection memory.
    if let Some(setup) = on_connect {
        runtime.arm_setup(setup().await);
    }

    // Keep the processing result instead of propagating it with `?`. An early
    // return here would skip finalization and leak per-connection resources on
    // decode, protocol, or transport errors.
    let processing_result = runtime.run(on_disconnect).await;

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

#[cfg(test)]
mod tests;
