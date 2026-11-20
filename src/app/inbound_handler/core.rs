//! Shared frame and stream processing for prepared application routes.
//!
//! [`process_stream`] is the entry point once a transport is framed. The
//! application path reaches it through `ConnectionRuntime::run`, after the
//! setup hook has run. The server's TCP path reaches it through
//! `PreparedApp::handle_connection_with_shutdown`, which runs
//! `process_connection` for the accepted socket. The connection spawner's own
//! `process_stream` is a different function: it completes the preamble
//! handshake and hands the socket to the prepared application, so it never
//! calls the frame loop directly. This function builds a [`ConnectionLoop`],
//! which owns the read half, the buffered inbound state and the response actor,
//! and runs it until the peer closes, the server stops, or a transport error
//! ends the connection. Each iteration applies memory pressure, waits for a
//! push batch or an inbound frame, then acts on it; the sibling modules named
//! in the parent overview own each of those steps.
//!
//! Push queues and the stop token arrive through [`StreamProcessingContext`]:
//! the queue template the actor clones into these lanes, the server shutdown
//! token, from which the loop derives a child token so its own exit cancels
//! only this connection, the [`SessionRegistry`] the actor registers with, and
//! the peer address diagnostics report. The loop observes shutdown between
//! units, so a handler kept alive by its own pushes still ends when the server
//! stops.
//!
//! A decoder failure is not an end of stream. The framed transport reports it
//! as an in-band [`DecodedItem::Failure`], and the loop's
//! [`RecoveryCoordinator`] asks the application's recovery hook what to do:
//! discard the frame, quarantine the connection briefly, or disconnect with the
//! decoder's own error. A failure buffered behind a response that ends the
//! connection is reported by [`settle_dispatch`], and one still buffered when
//! reading stops by [`report_pending_failure`], so every failure is observed
//! exactly once even when no policy can act on it.

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

// The modules below re-export the names their sibling steps use, so the loop's
// own file imports only what its definition, construction and read wait need.
use super::{
    super::{
        codec_driver::FramePipeline,
        envelope::{Envelope, Packet},
        frame_handling,
        memory_budgets::MemoryBudgets,
    },
    actor::{
        CancellationGuard,
        ConnectionActorInit,
        ConnectionResponseActor,
        build_connection_actor,
        observe_peer,
    },
    frames::InboundFrames,
    framing::{TransportSink, TransportStream, prepare_connection_framing},
    push_service::{StreamStep, StreamWait, next_stream_step},
    recovery::{RecoveryCoordinator, report_pending_failure},
};
use crate::{
    codec::{FrameCodec, RecoveryConfig, RecoveryPolicyHook},
    frame::FrameMetadata,
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    push::PushQueuesBuilder,
    serializer::Serializer,
    session::SessionRegistry,
};
// The sibling step modules are inherent methods on the loop and reach its
// private state, so they live in sibling files to keep this module within the
// size limit. They import the shared vocabulary below rather than repeating
// its crate paths.
#[path = "core_dispatch.rs"]
mod dispatch_steps;
#[path = "core_pushes.rs"]
mod push_steps;
// The memory-pressure exit has to drive the loop's own `step`, because the
// failure it reports never reaches the read or dispatch steps.
#[cfg(test)]
#[path = "core_pressure_tests.rs"]
mod pressure_tests;

pub(super) use super::{
    super::{PushWriteDiagnostics, frame_handling::Dispatch, recovery_codec::DecodedItem},
    dispatch::{FrameHandlingContext, handle_frame},
    frames::InboundExpiry,
    peer_watch::{PeerObservation, handle_watching_peer},
    push_service::{PushBatch, PushTransport, deliver_pushes, drain_queued},
    recovery::{resolve_read_item, settle_dispatch},
};
pub(super) use crate::{
    connection::PendingPushes,
    message::{DecodeWith, EncodeWith},
};

/// Immutable stream-wide configuration shared by all inbound frames.
pub(crate) struct StreamProcessingContext<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    pub(crate) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Serializer shared by all frames on the connection.
    pub(crate) serializer: Arc<S>,
    /// Codec configuration shared by all frames on the connection.
    pub(crate) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(crate) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Fragmentation settings used to initialize the frame pipeline.
    pub(crate) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Protocol hooks applied to every outbound frame on this connection.
    pub(crate) protocol_hooks: ProtocolHooks<F::Frame, ()>,
    /// Protocol whose response hooks drive the connection actor.
    pub(crate) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional byte budgets enforced while processing the connection.
    pub(crate) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(crate) read_timeout_ms: u64,
    /// Server shutdown signal; cancelling it stops this connection.
    pub(crate) shutdown: &'a CancellationToken,
    /// Template cloned to build this connection's push queues.
    pub(crate) push_queues: PushQueuesBuilder<F::Frame>,
    /// Registry this connection publishes its push handle to.
    pub(crate) session_registry: Arc<SessionRegistry<F::Frame>>,
    /// Peer address of the accepted socket, when the server reported one.
    pub(crate) peer_addr: Option<SocketAddr>,
    /// Optional hook consulted when a decoder fails on this connection.
    pub(crate) recovery_hook: Option<&'a dyn RecoveryPolicyHook>,
    /// Global recovery settings copied for this connection.
    pub(crate) recovery_config: RecoveryConfig,
}

/// Per-connection state driven by the inbound read loop.
struct ConnectionLoop<'a, S, E, F, W>
where
    S: Serializer,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Write half of the connection's framed transport.
    sink: TransportSink<W, F>,
    /// Read half of the connection's framed transport.
    reader: TransportStream<W, F>,
    /// Per-connection codec clone retaining framing state.
    codec: F,
    /// Buffered inbound item and the state a read timeout purges.
    inbound: InboundFrames<DecodedItem<F::Frame>, F::Frame>,
    /// Consecutive deserialization failures seen on this connection.
    deser_failures: u32,
    /// Serializer shared with the actor and the transport sink.
    serializer: Arc<S>,
    /// Middleware chains applied to decoded envelopes.
    routes: &'a HashMap<u32, HandlerService<E>>,
    /// Optional assembler consulted while handling frames.
    message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Protocol whose response and terminator hooks follow each response.
    protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Byte budgets enforced while reading.
    budgets: MemoryBudgets,
    /// Maximum interval to wait for the next inbound frame.
    read_timeout_ms: u64,
    /// Token whose cancellation stops this connection's inbound read.
    shutdown: CancellationToken,
    /// Hook consultation and drop accounting for decoder failures.
    recovery: RecoveryCoordinator<'a>,
    /// Whether the next wait should poll the inbound read before the queues.
    ///
    /// Set after a push batch so a continuously ready push lane cannot win
    /// every biased pass and starve the client's frames.
    serve_read_first: bool,
    /// Actor that owns outbound ordering for this connection.
    response_actor: ConnectionResponseActor<F>,
    /// Cancelled only by a clean EOF, which is permanent for the connection.
    peer_eof: CancellationToken,
    /// Cancels the connection token when this state is dropped.
    _cancel_on_exit: CancellationGuard,
}

impl<'a, S, E, F, W> ConnectionLoop<'a, S, E, F, W>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Prepare framing, assembly, budgets, and the output actor for `stream`.
    fn new(stream: W, context: StreamProcessingContext<'a, S, E, F>) -> io::Result<Self> {
        let StreamProcessingContext {
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
            recovery_hook,
            recovery_config,
        } = context;
        let (sink, reader, connection_codec) = prepare_connection_framing(stream, codec).split();
        let requested_frame_length = connection_codec.max_frame_length();
        let budgets =
            frame_handling::resolve_effective_budgets(memory_budgets, requested_frame_length);
        let mut inbound = InboundFrames {
            // A frame read while the previous one was still being handled, so
            // the loop never polls the read half twice for one inbound frame.
            pending: None,
            pipeline: FramePipeline::with_hooks(fragmentation, protocol_hooks),
            message_assembly: message_assembler.map(|_| {
                frame_handling::new_message_assembly_state(
                    fragmentation,
                    requested_frame_length,
                    Some(budgets),
                )
            }),
            // Armed on the first silent read, so a fresh connection never
            // starts by purging state it has not buffered.
            read_deadline: None,
        };
        // A child token so the guard below cancels only this connection, while
        // a server shutdown still reaches it through the parent.
        let connection_shutdown = shutdown.child_token();
        let read_shutdown = connection_shutdown.clone();
        let peer_eof = CancellationToken::new();
        let (response_actor, cancel_on_exit) = build_connection_actor(ConnectionActorInit {
            protocol: protocol.as_ref(),
            shutdown: connection_shutdown,
            registry: &session_registry,
            push_queues: &push_queues,
            context: ConnectionContext::default().with_peer_addr(peer_addr),
        })?;
        // The identifier is the one the actor claimed when it published its
        // handle, so the sink's callbacks and the registry agree on it.
        let connection_context =
            ConnectionContext::with_connection_id(Some(response_actor.connection_id))
                .with_peer_addr(peer_addr);
        // The sink-owned pipeline runs `before_send` too, so it must know
        // which connection the callback is running for.
        inbound.pipeline.set_context(connection_context);
        Ok(Self {
            sink,
            reader,
            codec: connection_codec,
            inbound,
            deser_failures: 0,
            serializer,
            routes,
            message_assembler,
            protocol,
            budgets,
            read_timeout_ms,
            shutdown: read_shutdown,
            recovery: RecoveryCoordinator::new(recovery_hook, recovery_config)
                .with_peer_address(peer_addr),
            serve_read_first: false,
            response_actor,
            peer_eof,
            _cancel_on_exit: cancel_on_exit,
        })
    }

    /// Read and process frames until the stream closes or an I/O error occurs.
    async fn run(mut self) -> io::Result<()> {
        let result = self.drive().await;
        self.report_buffered_failure();
        result
    }

    /// Report a failure still buffered when reading stops.
    ///
    /// Reading can stop for a reason that is not the failure itself: memory
    /// pressure, a cancelled read, or a response that ends the connection.
    /// Reporting it here is the only observation such a failure receives.
    fn report_buffered_failure(&mut self) {
        report_pending_failure(&mut self.inbound, &self.recovery);
    }

    /// Perform inbound work until the connection ends.
    async fn drive(&mut self) -> io::Result<()> {
        // Each iteration performs one unit of work, so the loop body is empty;
        // `step` reports whether the connection may continue.
        while self.step().await? {}
        Ok(())
    }

    /// Perform one unit of inbound work, reporting whether to continue.
    async fn step(&mut self) -> io::Result<bool> {
        if let Err(error) = self.inbound.relieve_pressure(self.budgets).await {
            // Memory pressure ends the connection before another read, so a
            // failure buffered from an earlier pass would otherwise be lost
            // behind the pressure error. Reporting it first keeps the
            // once-per-failure counter true for every failure.
            self.report_buffered_failure();
            return Err(error);
        }
        let Some(step) = self.wait_for_step().await? else {
            return Ok(false);
        };
        self.serve_read_first = matches!(&step, StreamStep::Pushes(_));
        self.handle_step(step).await
    }

    /// Wait for the next unit of work, or `None` once the server has stopped.
    ///
    /// The wait is raced against the connection's shutdown token. The read
    /// observes shutdown itself, but the push arm is polled first and a ready
    /// lane wins every biased pass, so without this arm a busy push stream
    /// would hide the stop from the connection task and the server drain
    /// waiting on it.
    async fn wait_for_step(&mut self) -> io::Result<Option<StreamStep<F, DecodedItem<F::Frame>>>> {
        tokio::select! {
            biased;
            () = self.shutdown.cancelled() => Ok(None),
            step = next_stream_step(
                &mut self.response_actor.actor,
                StreamWait {
                    inbound: &mut self.inbound,
                    reader: &mut self.reader,
                    read_timeout_ms: self.read_timeout_ms,
                    shutdown: &self.shutdown,
                    read_first: self.serve_read_first,
                },
            ) => step.map(Some),
        }
    }
}

/// Read and process frames until the stream closes or an I/O error occurs.
pub(crate) async fn process_stream<S, E, F, W>(
    stream: W,
    context: StreamProcessingContext<'_, S, E, F>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    ConnectionLoop::new(stream, context)?.run().await
}
