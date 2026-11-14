//! Shared frame and stream processing for prepared application routes.

use std::{collections::HashMap, sync::Arc};

use log::{debug, warn};
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
    frames::InboundFrames,
    framing::{TransportSink, TransportStream, prepare_connection_framing},
    peer_watch::{PeerObservation, handle_watching_peer},
    push_service::{PushTransport, StreamStep, StreamWait, next_stream_step},
};
use crate::{
    codec::FrameCodec,
    connection::PEER_CLOSED_RESPONSE_GRACE,
    frame::FrameMetadata,
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
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
    pub(super) push_queues: PushQueuesBuilder<F::Frame>,
    /// Registry this connection publishes its push handle to.
    pub(super) session_registry: Arc<SessionRegistry<F::Frame>>,
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
        };
        // A child token so the guard below cancels only this connection, while
        // a server shutdown still reaches it through the parent.
        let connection_shutdown = shutdown.child_token();
        let read_shutdown = connection_shutdown.clone();
        let peer_eof = CancellationToken::new();
        let connection_id = session_registry.next_connection_id();
        let connection_context = ConnectionContext::with_connection_id(Some(connection_id));
        // The sink-owned pipeline runs `before_send` too, so it must know
        // which connection the callback is running for.
        inbound.pipeline.set_context(connection_context.clone());
        let (response_actor, cancel_on_exit) = build_connection_actor(ConnectionActorInit {
            protocol: protocol.as_ref(),
            shutdown: connection_shutdown,
            connection_id,
            registry: &session_registry,
            push_queues: &push_queues,
            context: connection_context,
        })?;
        let idle_timeout = Duration::from_millis(read_timeout_ms);
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
            serve_read_first: false,
            response_actor,
            peer_eof,
            _cancel_on_exit: cancel_on_exit,
        })
    }

    /// Read and process frames until the stream closes or an I/O error occurs.
    async fn run(mut self) -> io::Result<()> {
        // Each iteration performs one unit of work, so the loop body is empty;
        // `step` reports whether the connection may continue.
        while self.step().await? {}
        Ok(())
    }

    /// Perform one unit of inbound work, reporting whether to continue.
    async fn step(&mut self) -> io::Result<bool> {
        self.inbound.relieve_pressure(self.budgets).await?;
        let step: StreamStep<F> = next_stream_step(
            &mut self.response_actor.actor,
            StreamWait {
                inbound: &mut self.inbound,
                reader: &mut self.reader,
                read_timeout_ms: self.read_timeout_ms,
                shutdown: &self.shutdown,
                read_first: self.serve_read_first,
            },
        )
        .await?;
        self.serve_read_first = matches!(&step, StreamStep::Pushes(_));
        match step {
            StreamStep::Pushes(frames) => {
                self.write_pushes(frames).await?;
                Ok(true)
            }
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
            StreamStep::Frame(frame) => {
                self.restart_idle_deadline();
                Ok(self.dispatch(frame).await? == Dispatch::Continue)
            }
        }
    }

    /// Start the idle interval again after the reader made progress.
    fn restart_idle_deadline(&mut self) { self.idle_deadline = Instant::now() + self.idle_timeout; }

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
                serializer: self.serializer.as_ref(),
                codec: &self.codec,
                message_assembler: self.message_assembler,
                message_assembly: &mut self.inbound.message_assembly,
                pipeline: &mut self.inbound.pipeline,
                actor: &mut self.response_actor.actor,
                protocol: self.protocol.as_ref(),
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
    async fn write_pushes(&mut self, frames: Vec<F::Frame>) -> io::Result<()> {
        let mut transport = PushTransport {
            serializer: self.serializer.as_ref(),
            codec: &self.codec,
            sink: &mut self.sink,
            pipeline: &mut self.inbound.pipeline,
        };
        transport.write(frames).await
    }

    /// Deliver frames still queued when the peer stopped writing.
    ///
    /// A peer that closed its write half but stopped reading applies
    /// back-pressure that can hold a send pending forever. The drain is
    /// therefore bounded by the connection's shutdown token and by the same
    /// grace that bounds a response whose peer can no longer be observed;
    /// either bound abandons the remaining frames instead of pinning the
    /// connection task and its registry entry.
    async fn drain_pushes(&mut self) -> io::Result<()> {
        let shutdown = self.response_actor.actor.shutdown_token();
        let mut transport = PushTransport {
            serializer: self.serializer.as_ref(),
            codec: &self.codec,
            sink: &mut self.sink,
            pipeline: &mut self.inbound.pipeline,
        };
        let drain = transport.drain(&mut self.response_actor.actor);
        tokio::select! {
            biased;
            result = drain => result,
            () = shutdown.cancelled() => Ok(()),
            () = tokio::time::sleep(PEER_CLOSED_RESPONSE_GRACE) => {
                warn!("abandoning queued pushes: the peer stopped reading");
                Ok(())
            }
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