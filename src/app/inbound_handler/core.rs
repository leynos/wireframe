//! Shared frame and stream processing for prepared application routes.

use std::{collections::HashMap, sync::Arc};

use futures::StreamExt;
use log::{debug, warn};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    time::{Duration, timeout},
};
use tokio_util::{codec::Framed, sync::CancellationToken};

use super::{
    super::{
        codec_driver::FramePipeline,
        combined_codec::ConnectionCodec,
        envelope::{Envelope, Packet},
        frame_handling,
        memory_budgets::MemoryBudgets,
    },
    MAX_DESER_FAILURES,
    actor::build_connection_actor,
    framing::{ConnectionFraming, prepare_connection_framing},
};
use crate::{
    codec::FrameCodec,
    connection::ConnectionActor,
    frame::FrameMetadata,
    hooks::{ProtocolHooks, WireframeProtocol},
    message::{DecodeWith, DeserializeContext, EncodeWith},
    message_assembler::{MessageAssembler, MessageAssemblyState},
    middleware::HandlerService,
    serializer::Serializer,
};

/// Per-frame processing state bundled for `handle_frame`.
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
    /// Actor that schedules and delivers handler responses.
    actor: &'a mut ConnectionActor<Envelope, ()>,
}

/// Immutable stream-wide configuration shared by all inbound frames.
pub(super) struct StreamProcessingContext<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    pub(super) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Serializer shared by all frames on the connection.
    pub(super) serializer: Arc<S>,
    /// Codec configuration shared by all frames on the connection.
    pub(super) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(super) message_assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Fragmentation settings used to initialize the frame pipeline.
    pub(super) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Protocol hooks applied to every outbound frame on this connection.
    pub(super) protocol_hooks: ProtocolHooks<F::Frame, ()>,
    /// Protocol whose response hooks drive the connection actor.
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional byte budgets enforced while processing the connection.
    pub(super) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(super) read_timeout_ms: u64,
}

/// State needed to turn a raw frame into a dispatchable envelope.
struct DispatchBuildContext<'a, F>
where
    F: FrameCodec,
{
    /// Raw frame borrowed while decoding its envelope metadata and payload.
    frame: &'a F::Frame,
    /// Pipeline needed to reassemble fragmented input and emit responses.
    pipeline: &'a mut FramePipeline<F::Frame>,
    /// Mutable assembly state retained across inbound frames.
    message_assembly: &'a mut Option<MessageAssemblyState>,
    /// Failure counter used to enforce the malformed-input limit.
    deser_failures: &'a mut u32,
}

/// Remove stale outbound and message-assembly state after an idle interval.
fn purge_expired<T>(
    pipeline: &mut FramePipeline<T>,
    message_assembly: &mut Option<MessageAssemblyState>,
) {
    pipeline.purge_expired();
    frame_handling::purge_expired_assemblies(message_assembly);
}

/// Apply memory pressure before reading the next inbound frame.
///
/// The read pauses for the configured interval while soft limits are exceeded,
/// and expired outbound and assembly state is purged on every pass.
async fn relieve_memory_pressure<T>(
    pipeline: &mut FramePipeline<T>,
    message_assembly: &mut Option<MessageAssemblyState>,
    budgets: MemoryBudgets,
) -> io::Result<()> {
    let pressure = frame_handling::evaluate_memory_pressure(
        message_assembly.as_ref(),
        Some(budgets),
        pipeline.fragmentation_buffered_bytes(),
    );
    frame_handling::apply_memory_pressure(pressure, || {
        purge_expired(pipeline, message_assembly);
    })
    .await
}

/// Detect a bincode budget failure so fallback decoding does not repeat the work.
fn is_bincode_limit_exceeded(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<bincode::error::DecodeError>()
        .is_some_and(|decode_error| {
            matches!(decode_error, bincode::error::DecodeError::LimitExceeded)
        })
}

/// Decode exactly one envelope, using metadata to select full deserialization.
pub(super) fn parse_envelope<S>(
    serializer: &S,
    payload: &[u8],
) -> std::result::Result<(Envelope, usize), Box<dyn std::error::Error + Send + Sync>>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    Envelope: DecodeWith<S>,
{
    let (envelope, consumed) = match serializer.parse(payload) {
        Ok((parsed_envelope, metadata_bytes_consumed)) => {
            if serializer.should_deserialize_after_parse() {
                let context = DeserializeContext {
                    frame_metadata: payload.get(..metadata_bytes_consumed),
                    message_id: Some(parsed_envelope.id),
                    correlation_id: parsed_envelope.correlation_id,
                    metadata_bytes_consumed: Some(metadata_bytes_consumed),
                };
                serializer.deserialize_with_context::<Envelope>(payload, &context)
            } else {
                Ok((parsed_envelope, metadata_bytes_consumed))
            }
        }
        Err(error) if is_bincode_limit_exceeded(&error) => {
            Err(Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
        }
        Err(_) => serializer.deserialize::<Envelope>(payload),
    }?;
    if consumed != payload.len() {
        crate::metrics::inc_decode_rejection(
            crate::metrics::DecodeRejectionPath::Envelope,
            crate::metrics::DecodeRejectionReason::TrailingBytes,
        );
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trailing bytes after envelope: frame must contain exactly one value",
        )
        .into());
    }
    Ok((envelope, consumed))
}

/// Read and process frames until the stream closes or an I/O error occurs.
pub(super) async fn process_stream<S, E, F, W>(
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
    } = context;
    let ConnectionFraming {
        codec: connection_codec,
        mut framed,
    } = prepare_connection_framing(stream, codec);
    let requested_frame_length = connection_codec.max_frame_length();
    let effective_budgets =
        frame_handling::resolve_effective_budgets(memory_budgets, requested_frame_length);
    let mut deser_failures = 0u32;
    let mut message_assembly = message_assembler.map(|_| {
        frame_handling::new_message_assembly_state(
            fragmentation,
            requested_frame_length,
            Some(effective_budgets),
        )
    });
    let mut pipeline = FramePipeline::with_hooks(fragmentation, protocol_hooks);
    let connection_shutdown = CancellationToken::new();
    let (mut response_actor, _cancel_on_exit) = build_connection_actor::<S, F>(
        protocol.as_ref(),
        Arc::clone(&serializer),
        connection_shutdown,
    )?;

    loop {
        relieve_memory_pressure(&mut pipeline, &mut message_assembly, effective_budgets).await?;

        match timeout(Duration::from_millis(read_timeout_ms), framed.next()).await {
            Ok(Some(Ok(frame))) => {
                handle_frame(
                    &frame,
                    FrameHandlingContext {
                        framed: &mut framed,
                        deser_failures: &mut deser_failures,
                        routes,
                        serializer: serializer.as_ref(),
                        codec: &connection_codec,
                        message_assembler,
                        message_assembly: &mut message_assembly,
                        pipeline: &mut pipeline,
                        actor: &mut response_actor.actor,
                    },
                )
                .await?;
            }
            Ok(Some(Err(error))) => return Err(error),
            Ok(None) => break,
            Err(_) => {
                debug!("read timeout elapsed; continuing to wait for next frame");
                purge_expired(&mut pipeline, &mut message_assembly);
            }
        }
    }

    Ok(())
}

/// Decode one frame, apply reassembly, and dispatch the resulting envelope.
async fn handle_frame<S, E, F, W>(
    frame: &F::Frame,
    context: FrameHandlingContext<'_, S, E, W, F>,
) -> io::Result<()>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin,
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
        actor,
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
                actor,
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

/// Build a dispatchable envelope through decode, reassembly, and assembly.
fn build_dispatchable_envelope<S, F>(
    serializer: &S,
    message_assembler: Option<&Arc<dyn MessageAssembler>>,
    context: DispatchBuildContext<'_, F>,
) -> io::Result<Option<Envelope>>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    F: FrameCodec,
    Envelope: DecodeWith<S>,
{
    let DispatchBuildContext {
        frame,
        pipeline,
        message_assembly,
        deser_failures,
    } = context;
    let mut failure_tracker =
        frame_handling::DeserFailureTracker::new(deser_failures, MAX_DESER_FAILURES);
    let Some(envelope) = frame_handling::decode_envelope::<F>(
        parse_envelope(serializer, F::frame_payload(frame)),
        frame,
        &mut failure_tracker,
    )?
    else {
        return Ok(None);
    };
    let Some(reassembled_envelope) = frame_handling::reassemble_if_needed(
        pipeline,
        deser_failures,
        envelope,
        MAX_DESER_FAILURES,
    )?
    else {
        return Ok(None);
    };
    let Some(assembled_envelope) = frame_handling::assemble_if_needed(
        frame_handling::AssemblyRuntime::new(message_assembler, message_assembly),
        deser_failures,
        reassembled_envelope,
        MAX_DESER_FAILURES,
    )?
    else {
        return Ok(None);
    };

    // Reset only after the entire pipeline succeeds, so assembly failures
    // accumulate towards the close threshold.
    *deser_failures = 0;
    Ok(Some(assembled_envelope))
}
