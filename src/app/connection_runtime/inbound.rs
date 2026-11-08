//! Mutable inbound state for exactly one accepted connection.

use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::StreamExt;
use log::{debug, warn};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    time::timeout,
};
use tokio_util::{codec::Framed, sync::CancellationToken};

use super::decode::{DispatchBuildContext, build_dispatchable_envelope};
use crate::{
    app::{
        codec_driver::FramePipeline,
        combined_codec::{CombinedCodec, ConnectionCodec},
        envelope::{Envelope, Packet},
        frame_handling,
        memory_budgets::MemoryBudgets,
    },
    codec::{FrameCodec, MAX_FRAME_LENGTH, clamp_frame_length},
    frame::FrameMetadata,
    hooks::ProtocolHooks,
    message::{DecodeWith, EncodeWith},
    message_assembler::{MessageAssembler, MessageAssemblyState},
    middleware::HandlerService,
    serializer::Serializer,
};

/// Immutable configuration borrowed by one connection's inbound state.
///
/// Every field is template-owned and shared across connections; none of it is
/// mutated while the connection runs.
pub(crate) struct InboundConfig<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    pub(crate) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Serializer shared by all frames on the connection.
    pub(crate) serializer: &'a S,
    /// Codec configuration shared by all frames on the connection.
    pub(crate) codec: &'a F,
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

/// Mutable state owned by one inbound connection.
///
/// This value is never shared between connection tasks: the stream, framed
/// codec state, frame pipeline, assembly state, and failure counter all have
/// exactly one owner for the connection's lifetime.
pub(crate) struct InboundState<'a, S, E, F, W>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    routes: &'a HashMap<u32, HandlerService<E>>,
    /// Serializer used to decode envelopes and encode responses.
    serializer: &'a S,
    /// Optional assembly strategy for multi-frame protocol messages.
    message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Codec clone that carries per-connection framing state.
    connection_codec: F,
    /// Framed stream built from the codec's decoder and encoder.
    framed: Framed<W, ConnectionCodec<F>>,
    /// Outbound processing state for fragmenting, hooking, and counting
    /// responses.
    pipeline: FramePipeline<F::Frame>,
    /// Connection-local state for assembling multi-frame messages.
    message_assembly: Option<MessageAssemblyState>,
    /// Consecutive malformed-envelope count for this connection.
    deser_failures: u32,
    /// Byte budgets resolved once from the prepared configuration.
    effective_budgets: MemoryBudgets,
    /// Deadline applied to each read of the next inbound frame.
    timeout_dur: Duration,
}

/// How one iteration of the inbound read loop finished.
enum InboundRead<Frame> {
    /// A complete frame is ready to dispatch.
    Frame(Frame),
    /// The peer closed the stream cleanly.
    Closed,
    /// The read timeout elapsed; processing continues.
    TimedOut,
    /// Cooperative cancellation was observed at the read boundary.
    Cancelled,
}

impl<'a, S, E, F, W> InboundState<'a, S, E, F, W>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite,
{
    /// Build connection-local framing and pipeline state over `stream`.
    pub(crate) fn new(stream: W, config: InboundConfig<'a, S, E, F>) -> Self {
        let InboundConfig {
            routes,
            serializer,
            codec,
            message_assembler,
            fragmentation,
            protocol_hooks,
            memory_budgets,
            read_timeout_ms,
        } = config;
        // Each connection needs isolated framing state: cloning resets the
        // counters `SeqFrameCodec` and `TaggedFrameCodec::wrap_payload`
        // consume.
        let connection_codec = codec.clone();
        let combined = CombinedCodec::new(connection_codec.decoder(), connection_codec.encoder());
        let mut framed = Framed::new(stream, combined);
        let requested_frame_length = connection_codec.max_frame_length();
        let max_frame_length = clamp_frame_length(requested_frame_length);
        if requested_frame_length > MAX_FRAME_LENGTH {
            warn!(
                "codec max frame length exceeds guardrail; clamping to {MAX_FRAME_LENGTH} bytes \
                 (requested={requested_frame_length})"
            );
        }
        framed.read_buffer_mut().reserve(max_frame_length);
        let effective_budgets =
            frame_handling::resolve_effective_budgets(memory_budgets, requested_frame_length);
        let message_assembly = message_assembler.map(|_| {
            frame_handling::new_message_assembly_state(
                fragmentation,
                requested_frame_length,
                Some(effective_budgets),
            )
        });
        let pipeline = FramePipeline::with_hooks(fragmentation, protocol_hooks);
        Self {
            routes,
            serializer,
            message_assembler,
            connection_codec,
            framed,
            pipeline,
            message_assembly,
            deser_failures: 0,
            effective_budgets,
            timeout_dur: Duration::from_millis(read_timeout_ms),
        }
    }

    /// Read and process frames until the stream closes, fails, or is
    /// cooperatively cancelled.
    ///
    /// Cancellation is observed only at the read boundary. A frame that has
    /// already been read is decoded, reassembled, and dispatched to completion
    /// before the next loop iteration can notice cancellation, which keeps
    /// handler work and response writes from being torn in half.
    pub(crate) async fn run(&mut self, cancellation: Option<&CancellationToken>) -> io::Result<()>
    where
        S: FrameMetadata<Frame = Envelope>,
        W: Unpin,
        Envelope: DecodeWith<S> + EncodeWith<S>,
    {
        loop {
            let pressure = frame_handling::evaluate_memory_pressure(
                self.message_assembly.as_ref(),
                Some(self.effective_budgets),
                self.pipeline.fragmentation_buffered_bytes(),
            );
            frame_handling::apply_memory_pressure(pressure, || self.purge_expired()).await?;

            let read = tokio::select! {
                biased;
                () = wait_for_cancellation(cancellation) => InboundRead::Cancelled,
                result = timeout(self.timeout_dur, self.framed.next()) => match result {
                    Ok(Some(Ok(frame))) => InboundRead::Frame(frame),
                    Ok(Some(Err(error))) => return Err(error),
                    Ok(None) => InboundRead::Closed,
                    Err(_) => InboundRead::TimedOut,
                },
            };

            match read {
                InboundRead::Frame(frame) => {
                    handle_frame(
                        &frame,
                        FrameHandlingContext {
                            framed: &mut self.framed,
                            deser_failures: &mut self.deser_failures,
                            routes: self.routes,
                            serializer: self.serializer,
                            codec: &self.connection_codec,
                            message_assembler: self.message_assembler,
                            message_assembly: &mut self.message_assembly,
                            pipeline: &mut self.pipeline,
                        },
                    )
                    .await?;
                }
                InboundRead::Closed => break,
                InboundRead::TimedOut => {
                    debug!("read timeout elapsed; continuing to wait for next frame");
                    self.purge_expired();
                }
                InboundRead::Cancelled => return Ok(()),
            }
        }

        Ok(())
    }

    /// Remove stale outbound and message-assembly state after an idle
    /// interval or a memory-pressure pause.
    fn purge_expired(&mut self) {
        self.pipeline.purge_expired();
        frame_handling::purge_expired_assemblies(&mut self.message_assembly);
    }
}

/// Wait until cancellation is requested, or forever when no token is supplied.
async fn wait_for_cancellation(cancellation: Option<&CancellationToken>) {
    match cancellation {
        Some(token) => token.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

/// Per-frame processing state bundled for [`handle_frame`].
struct FrameHandlingContext<'a, S, E, W, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
{
    /// Framed transport used to write responses during frame handling.
    framed: &'a mut Framed<W, ConnectionCodec<F>>,
    /// Connection-wide malformed-frame counter shared with all stages.
    deser_failures: &'a mut u32,
    /// Immutable middleware chains used to dispatch decoded envelopes.
    routes: &'a HashMap<u32, HandlerService<E>>,
    /// Serializer used to decode envelopes and encode responses.
    serializer: &'a S,
    /// Codec configuration used by the response path.
    codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Outbound processing state for fragmenting, hooking, and counting
    /// responses.
    pipeline: &'a mut FramePipeline<F::Frame>,
    /// Connection-local state for assembling multi-frame messages.
    message_assembly: &'a mut Option<MessageAssemblyState>,
}

/// Decode one frame, apply reassembly, and dispatch the resulting envelope.
async fn handle_frame<S, E, F, W>(
    frame: &F::Frame,
    context: FrameHandlingContext<'_, S, E, W, F>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    let FrameHandlingContext {
        framed,
        deser_failures,
        routes,
        serializer,
        codec,
        message_assembler,
        message_assembly,
        pipeline,
    } = context;

    crate::metrics::inc_frames(crate::metrics::Direction::Inbound);
    let Some(envelope) = build_dispatchable_envelope(
        serializer,
        message_assembler,
        DispatchBuildContext::<F> {
            frame,
            pipeline,
            message_assembly,
            deser_failures,
        },
    )?
    else {
        return Ok(());
    };

    if let Some(service) = routes.get(&envelope.id) {
        frame_handling::forward_response(
            envelope,
            service,
            frame_handling::ResponseContext::<S, W, F> {
                serializer,
                framed,
                pipeline,
                codec,
            },
        )
        .await?;
    } else {
        warn!(
            "no handler for message id: id={}, correlation_id={:?}",
            envelope.id, envelope.correlation_id
        );
    }

    Ok(())
}
