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

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use log::debug;
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        codec_driver::FramePipeline,
        envelope::{Envelope, Packet},
        frame_handling::{self, Dispatch},
        memory_budgets::MemoryBudgets,
    },
    actor::{
        CancellationGuard,
        ConnectionActorInit,
        ConnectionResponseActor,
        build_connection_actor,
        observe_peer,
    },
    dispatch::{FrameHandlingContext, handle_frame},
    frames::{InboundExpiry, InboundFrames},
    framing::{TransportSink, TransportStream, prepare_connection_framing, report_truncation},
    peer_watch::{PeerObservation, handle_watching_peer},
    push_service::{
        PushBatch,
        PushTransport,
        StreamStep,
        StreamWait,
        deliver_pushes,
        drain_queued,
        next_stream_step,
    },
};
use crate::{
    codec::FrameCodec,
    connection::PendingPushes,
    frame::FrameMetadata,
    hooks::{ConnectionContext, ProtocolHooks},
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    push::PushQueuesBuilder,
    serializer::Serializer,
    session::SessionRegistry,
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
    /// Serializer borrowed from the prepared root for the connection lifetime.
    pub(crate) serializer: &'a S,
    /// Codec configuration shared by all frames on the connection.
    pub(crate) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(crate) message_assembler: Option<&'a dyn MessageAssembler>,
    /// Fragmentation settings used to initialize the frame pipeline.
    pub(crate) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Protocol hooks applied to every outbound frame on this connection.
    ///
    /// The hooks own the connection's single strong protocol reference. Setup
    /// dispatches through this value while it is still borrowed, and the same
    /// value then moves into the frame pipeline.
    pub(crate) protocol_hooks: ProtocolHooks<F::Frame, ()>,
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
    /// Buffered inbound frame and the state a read timeout purges.
    inbound: InboundFrames<F::Frame>,
    /// Consecutive deserialization failures seen on this connection.
    deser_failures: u32,
    /// Serializer borrowed for the connection lifetime.
    serializer: &'a S,
    /// Middleware chains applied to decoded envelopes.
    routes: &'a HashMap<u32, HandlerService<E>>,
    /// Optional assembler consulted while handling frames.
    message_assembler: Option<&'a dyn MessageAssembler>,
    /// Byte budgets enforced while reading.
    budgets: MemoryBudgets,
    /// Maximum interval to wait for the next inbound frame.
    read_timeout_ms: u64,
    /// Token whose cancellation stops this connection's inbound read.
    shutdown: CancellationToken,
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
            memory_budgets,
            read_timeout_ms,
            shutdown,
            push_queues,
            session_registry,
            peer_addr,
        } = context;
        let (sink, reader, connection_codec) = prepare_connection_framing(stream, codec).split();
        let requested_frame_length = connection_codec.max_frame_length();
        let budgets =
            frame_handling::resolve_effective_budgets(memory_budgets, requested_frame_length);
        // A child token so the guard below cancels only this connection, while
        // a server shutdown still reaches it through the parent.
        let connection_shutdown = shutdown.child_token();
        let read_shutdown = connection_shutdown.clone();
        let peer_eof = CancellationToken::new();
        // Protocol setup borrows the hooks value that will own this
        // connection's single protocol reference, then that same value moves
        // into the pipeline below. Cloning the protocol for setup would give
        // the connection a second strong reference while setup is pending.
        let mut protocol_hooks = protocol_hooks;
        let (response_actor, cancel_on_exit, connection_context) =
            build_connection_actor(ConnectionActorInit {
                protocol_hooks: &mut protocol_hooks,
                shutdown: connection_shutdown,
                registry: &session_registry,
                push_queues: &push_queues,
                context: ConnectionContext::default().with_peer_addr(peer_addr),
            })?;
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
// The sink-owned pipeline runs `before_send` too, so it must know
        // which connection the callback is running for. Installing the value
        // setup observed — rather than rebuilding an equal one — means state
        // setup leaves in the context carries into every later callback.
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
            budgets,
            read_timeout_ms,
            shutdown: read_shutdown,
            serve_read_first: false,
            response_actor,
            peer_eof,
            _cancel_on_exit: cancel_on_exit,
        })
    }

    /// Read and process frames until the stream closes or an I/O error occurs.
    ///
    /// Every error that ends the connection passes this one boundary, whichever
    /// step observed it; that is where a truncation is reported.
    async fn run(mut self) -> io::Result<()> {
        let outcome = self.run_steps().await;
        if let Err(error) = &outcome {
            // The reader still holds what the peer abandoned; report it first.
            report_truncation::<W, F>(&*self.reader, &mut self.inbound.pipeline, error);
        }
        outcome
    }

    /// Perform one unit of work at a time until the connection is finished.
    async fn run_steps(&mut self) -> io::Result<()> {
        while self.step().await? {}
        Ok(())
    }

    /// Perform one unit of inbound work, reporting whether to continue.
    async fn step(&mut self) -> io::Result<bool> {
        self.inbound.relieve_pressure(self.budgets).await?;
        let Some(step) = self.wait_for_step().await? else {
            return Ok(false);
        };
        self.serve_read_first = matches!(&step, StreamStep::Pushes(_));
        self.handle_step(step).await
    }

    /// Wait for the next unit of work, or `None` once the server has stopped.
    ///
    /// The read observes shutdown itself, but a ready push lane wins every
    /// biased pass, so the race below keeps a busy push stream from hiding
    /// the stop from the server drain waiting on this connection task.
    async fn wait_for_step(&mut self) -> io::Result<Option<StreamStep<F>>> {
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

    /// Act on one unit of work, reporting whether the connection continues.
    async fn handle_step(&mut self, step: StreamStep<F>) -> io::Result<bool> {
        match step {
            StreamStep::Pushes(pending) => self.write_pushes(pending).await,
            StreamStep::Idle => {
                debug!("push lanes closed; continuing to wait for the next frame");
                Ok(true)
            }
            StreamStep::Closed => {
                // Frames already queued are still delivered; anything pushed
                // afterwards is rejected because the actor closes both lanes.
                self.drain_pushes().await?;
                Ok(false)
            }
            // A failed or cancelled response leaves its stream without a
            // terminator, so a later response would be read as part of the
            // truncated stream. Stop instead.
            StreamStep::Frame(frame) => Ok(self.dispatch(frame).await? == Dispatch::Continue),
        }
    }

    /// Decode, reassemble, and dispatch one inbound frame while watching the
    /// peer for a disconnect.
    async fn dispatch(&mut self, frame: F::Frame) -> io::Result<Dispatch> {
        let (unobserved, cause) = observe_peer(&mut self.response_actor.actor, &self.peer_eof);
        let handling = handle_frame(
            &frame,
            FrameHandlingContext {
                sink: &mut self.sink,
                deser_failures: &mut self.deser_failures,
                routes: self.routes,
                serializer: self.serializer,
                codec: &self.codec,
                message_assembler: self.message_assembler,
                message_assembly: &mut self.inbound.message_assembly,
                pipeline: &mut self.inbound.pipeline,
                actor: &mut self.response_actor.actor,
            },
        );
        handle_watching_peer(
            handling,
            &mut self.reader,
            PeerObservation::new(&self.peer_eof, &unobserved, &cause),
            &mut self.inbound.pending,
        )
        .await
    }

    /// Write queued push frames through the connection's transport sink.
    ///
    /// A write to a peer that stopped reading can stay pending indefinitely,
    /// so it is bounded by the shutdown token and, once the peer can no longer
    /// be observed, by the idle grace. The read half keeps being polled while
    /// the write runs, because this is the last place that can notice a peer
    /// which half-closed and then stopped reading.
    async fn write_pushes(&mut self, pending: PendingPushes<F::Frame>) -> io::Result<bool> {
        let mut transport = PushTransport {
            serializer: self.serializer,
            codec: &self.codec,
            sink: &mut self.sink,
            pipeline: &mut self.inbound.pipeline,
            diagnostics: self.response_actor.push_diagnostics(),
        };
        let batch = PushBatch {
            frames: pending.frames,
            priority: pending.priority,
            reader: &mut self.reader,
            pending_frame: &mut self.inbound.pending,
            expiry: InboundExpiry::new(
                &mut self.inbound.message_assembly,
                &mut self.inbound.read_deadline,
                self.read_timeout_ms,
            ),
            peer_eof: &self.peer_eof,
            shutdown: &self.shutdown,
        };
        deliver_pushes(&mut transport, &mut self.response_actor.actor, batch).await
    }

    /// Deliver frames still queued when the peer stopped writing.
    async fn drain_pushes(&mut self) -> io::Result<()> {
        let mut transport = PushTransport {
            serializer: self.serializer,
            codec: &self.codec,
            sink: &mut self.sink,
            pipeline: &mut self.inbound.pipeline,
            diagnostics: self.response_actor.push_diagnostics(),
        };
        drain_queued(
            &mut transport,
            &mut self.response_actor.actor,
            &self.shutdown,
        )
        .await
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
