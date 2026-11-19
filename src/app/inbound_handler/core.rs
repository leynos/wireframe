//! Shared frame and stream processing for prepared application routes.

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use super::{
    super::{
        envelope::{Envelope, Packet},
        frame_handling::Dispatch,
        memory_budgets::MemoryBudgets,
        recovery_codec::DecodedItem,
    },
    actor::{ConnectionRuntime, RuntimeParams, observe_peer},
    dispatch::{FrameHandlingContext, handle_frame},
    frames::InboundFrames,
    framing::{
        TransportStream,
        prepare_connection_framing,
        prepare_inbound_state,
        resolve_framing_limits,
    },
    peer_watch::{PeerObservation, handle_watching_peer},
    recovery::{report_pending_failure, resolve_read_item, settle_dispatch},
};
use crate::{
    codec::{FrameCodec, RecoveryConfig, RecoveryPolicyHook},
    frame::FrameMetadata,
    hooks::{ProtocolHooks, WireframeProtocol},
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    serializer::Serializer,
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
    /// Optional hook consulted when a decoder fails on this connection.
    pub(crate) recovery_hook: Option<&'a dyn RecoveryPolicyHook>,
    /// Global recovery settings copied for this connection.
    pub(crate) recovery_config: RecoveryConfig,
    /// Peer address of the accepted connection, when the transport reported one.
    pub(crate) peer_address: Option<SocketAddr>,
}

impl<'a, S, E, F> StreamProcessingContext<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// The per-connection handles this stream asks its runtime to build.
    fn runtime_params(&self) -> RuntimeParams<'a, F> {
        RuntimeParams {
            protocol: self.protocol.clone(),
            server_shutdown: self.shutdown,
            recovery_hook: self.recovery_hook,
            recovery_config: self.recovery_config.clone(),
            peer_address: self.peer_address,
        }
    }
}

/// How one connection waits for its next inbound frame.
///
/// These values are resolved once per connection, so the read step names them
/// together rather than threading three arguments through the loop.
#[derive(Clone, Copy)]
struct ReadPoll<'a> {
    /// Maximum interval to wait for the next inbound frame.
    read_timeout_ms: u64,
    /// Server shutdown signal observed while waiting.
    shutdown: &'a CancellationToken,
    /// Resolved byte budgets applied before each read.
    budgets: MemoryBudgets,
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
    let (mut sink, mut reader, connection_codec) =
        prepare_connection_framing(stream, context.codec).split();
    let limits = resolve_framing_limits(context.memory_budgets, &connection_codec);
    // Built before the inbound state takes ownership of the protocol hooks, so
    // the runtime can read the remaining stream configuration as a whole.
    let mut runtime = ConnectionRuntime::prepare(context.runtime_params())?;
    let mut deser_failures = 0u32;
    let mut inbound = prepare_inbound_state::<F>(
        context.fragmentation,
        context.protocol_hooks,
        context.message_assembler,
        limits,
    );
    let poll = ReadPoll {
        read_timeout_ms: context.read_timeout_ms,
        shutdown: context.shutdown,
        budgets: limits.effective_budgets,
    };

    loop {
        let Some(frame) = next_frame::<W, F>(&mut inbound, &mut reader, &mut runtime, poll).await?
        else {
            break;
        };

        let (unobserved, cause) =
            observe_peer(&mut runtime.response_actor.actor, &runtime.peer_eof);
        let dispatch = settle_dispatch(
            handle_watching_peer(
                handle_frame(
                    &frame,
                    FrameHandlingContext {
                        sink: &mut sink,
                        deser_failures: &mut deser_failures,
                        routes: context.routes,
                        serializer: context.serializer.as_ref(),
                        codec: &connection_codec,
                        message_assembler: context.message_assembler,
                        message_assembly: &mut inbound.message_assembly,
                        pipeline: &mut inbound.pipeline,
                        actor: &mut runtime.response_actor.actor,
                        protocol: context.protocol.as_ref(),
                    },
                ),
                &mut reader,
                PeerObservation::new(&runtime.peer_eof, &unobserved, &cause),
                &mut inbound.pending,
            )
            .await,
            &mut inbound,
            &runtime.recovery,
        )?;
        // A cancelled response left its stream without a terminator, so any
        // further response would be read as part of it. Drop the buffered
        // request and close the connection instead.
        if dispatch == Dispatch::Stop {
            break;
        }
    }

    Ok(())
}

/// Read the next dispatchable frame, applying recovery to decoder failures.
///
/// Returns `None` once the peer has closed or the server has stopped. A
/// discarded failure is not an end of stream: the policy kept the connection
/// alive, so this reads on and only the caller sees a frame that actually
/// decoded.
async fn next_frame<W, F>(
    inbound: &mut InboundFrames<DecodedItem<F::Frame>, F::Frame>,
    reader: &mut TransportStream<W, F>,
    runtime: &mut ConnectionRuntime<'_>,
    poll: ReadPoll<'_>,
) -> io::Result<Option<F::Frame>>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        inbound.relieve_pressure(poll.budgets).await?;
        let Some(item) = inbound
            .next(reader, poll.read_timeout_ms, poll.shutdown)
            .await?
        else {
            // Shutdown wins the read race before the buffered item is taken,
            // so the loop can end still holding a decoder failure.
            report_pending_failure(inbound, &runtime.recovery);
            return Ok(None);
        };
        if let Some(frame) =
            resolve_read_item(item, inbound, &mut runtime.recovery, &runtime.shutdown).await?
        {
            return Ok(Some(frame));
        }
    }
}

#[cfg(test)]
#[path = "read_loop_tests.rs"]
mod read_loop_tests;
