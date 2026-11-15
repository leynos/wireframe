//! Helpers for protocol-level message assembly on the inbound path.

use std::{collections::HashMap, io, num::NonZeroUsize, sync::Arc, time::Duration};

use bytes::Bytes;
use log::debug;

use super::{buffered_assembly, core::DeserFailureTracker, stream_assembly};
use crate::{
    app::{
        Envelope,
        StreamingHandler,
        builder_defaults::default_fragmentation,
        memory_budgets::MemoryBudgets,
    },
    codec::clamp_frame_length,
    fragment::FragmentationConfig,
    message_assembler::{
        AssembledMessage,
        EnvelopeRouting,
        FrameHeader,
        MessageAssembler,
        MessageAssemblyState,
        MessageKey,
        PurgedAssemblies,
    },
    request::RequestParts,
};

/// Default timeout used when no fragmentation-derived timeout is available.
const DEFAULT_MESSAGE_ASSEMBLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Borrowed inbound runtime state used for message assembly.
pub(crate) struct AssemblyRuntime<'a> {
    /// Shared assembler implementation, absent when assembly is disabled.
    pub(crate) assembler: Option<&'a Arc<dyn MessageAssembler>>,
    /// Mutable connection-local assembly state and its memory accounting.
    pub(crate) state: &'a mut Option<MessageAssemblyState>,
    /// Streaming route identifiers. Their first frames are delivered chunk by
    /// chunk when no other streaming body is live.
    pub(crate) streaming_routes: &'a HashMap<u32, StreamingHandler>,
    /// Whether another streaming body currently owns the dispatch slot.
    pub(crate) stream_live: bool,
}

impl<'a> AssemblyRuntime<'a> {
    /// Create assembly runtime accessors for one inbound frame.
    #[must_use]
    pub(crate) fn new(
        assembler: Option<&'a Arc<dyn MessageAssembler>>,
        state: &'a mut Option<MessageAssemblyState>,
        streaming_routes: &'a HashMap<u32, StreamingHandler>,
        stream_live: bool,
    ) -> Self {
        Self {
            assembler,
            state,
            streaming_routes,
            stream_live,
        }
    }
}

/// What one inbound frame produced for the connection loop.
#[derive(Debug)]
pub(crate) enum FrameOutcome {
    /// More frames are required, the frame was discarded, or a recoverable
    /// rejection was recorded.
    Pending,
    /// A complete message for a buffered route.
    Buffered(Envelope),
    /// A streaming message whose body stream is now owned by a handler.
    StreamStarted {
        /// Assembly key, absent when no assembler participates.
        key: Option<MessageKey>,
        /// Routing metadata captured from the first frame.
        parts: RequestParts,
        /// First body chunk.
        chunk: Bytes,
        /// Whether this chunk also completes the message.
        is_last: bool,
    },
    /// A continuation chunk for the live streaming message.
    StreamChunk {
        /// Message key the chunk belongs to.
        key: MessageKey,
        /// Body bytes carried by the frame.
        chunk: Bytes,
        /// Whether this chunk completes the message.
        is_last: bool,
    },
    /// A complete message for a streaming route that arrived while another
    /// stream was live. It is delivered as a pre-filled stream later.
    StreamFallback {
        /// Routing metadata for the parked request.
        parts: RequestParts,
        /// Fully assembled request body.
        body: Bytes,
    },
    /// A continuation was validated and dropped because the consumer
    /// abandoned the message.
    Discarded,
    /// A terminal streaming failure closed the message assembly.
    StreamFailed {
        /// Key whose assembly failed.
        key: MessageKey,
        /// Error delivered to the handler's body.
        error: io::Error,
    },
}

/// Build a connection-scoped message assembly state from known budgets.
///
/// When `memory_budgets` is `Some`, the effective per-message limit is
/// `min(fragmentation_max, bytes_per_message)` and the connection/in-flight
/// budgets are forwarded to [`MessageAssemblyState::with_budgets`].
#[must_use]
pub(crate) fn new_message_assembly_state(
    fragmentation: Option<FragmentationConfig>,
    frame_budget: usize,
    memory_budgets: Option<MemoryBudgets>,
) -> MessageAssemblyState {
    let config = fragmentation.or_else(|| default_fragmentation(frame_budget));
    let frag_max = config.map_or_else(
        || NonZeroUsize::new(clamp_frame_length(frame_budget)).unwrap_or(NonZeroUsize::MIN),
        |cfg| cfg.max_message_size,
    );
    let timeout = config.map_or(DEFAULT_MESSAGE_ASSEMBLY_TIMEOUT, |cfg| {
        cfg.reassembly_timeout
    });

    memory_budgets.map_or_else(
        || MessageAssemblyState::new(frag_max, timeout),
        |budgets| {
            let per_message = budgets.bytes_per_message().get();
            let max_message_size = frag_max.min(per_message);
            MessageAssemblyState::with_budgets(
                max_message_size,
                timeout,
                Some(budgets.bytes_per_connection().get()),
                Some(budgets.bytes_in_flight().get()),
            )
        },
    )
}

/// Purge stale in-flight assemblies.
///
/// Streaming keys are reported separately so the connection can fail the
/// waiting handler's body with a timeout instead of leaving it open forever.
#[must_use]
pub(crate) fn purge_expired_assemblies(
    assembly: &mut Option<MessageAssemblyState>,
) -> PurgedAssemblies {
    let Some(state) = assembly.as_mut() else {
        return PurgedAssemblies::default();
    };

    let evicted = state.purge_expired_detailed();
    if !evicted.is_empty() {
        debug!("purged expired message assemblies: {evicted:?}");
    }
    evicted
}

/// Apply protocol-level message assembly to a complete post-fragment envelope.
pub(crate) fn assemble_if_needed(
    runtime: AssemblyRuntime<'_>,
    deser_failures: &mut u32,
    env: Envelope,
    max_deser_failures: u32,
) -> io::Result<FrameOutcome> {
    let AssemblyRuntime {
        assembler: maybe_assembler,
        state: assembly,
        streaming_routes,
        stream_live,
    } = runtime;
    let Some(assembler) = maybe_assembler else {
        return Ok(fallback_outcome(env, streaming_routes, stream_live));
    };
    let Some(state) = assembly.as_mut() else {
        return Ok(fallback_outcome(env, streaming_routes, stream_live));
    };

    let mut failures = DeserFailureTracker::new(deser_failures, max_deser_failures);
    let correlation_id = env.correlation_id;

    let parsed = match assembler.parse_frame_header(env.payload_bytes()) {
        Ok(parsed) => parsed,
        Err(err) => {
            failures.record(
                correlation_id,
                "failed to parse message assembly frame header",
                err,
            )?;
            return Ok(FrameOutcome::Pending);
        }
    };

    let payload = env.payload_bytes();
    let Some(frame_bytes) = payload.get(parsed.header_len()..) else {
        failures.record(
            correlation_id,
            "message assembly header length exceeds payload length",
            io::Error::new(
                io::ErrorKind::InvalidData,
                "message assembly header length exceeds payload",
            ),
        )?;
        return Ok(FrameOutcome::Pending);
    };

    let routing = EnvelopeRouting {
        envelope_id: env.id.into(),
        correlation_id: env.correlation_id.map(Into::into),
    };

    match parsed.into_header() {
        FrameHeader::First(header) => {
            if streaming_routes.contains_key(&env.id) && !stream_live {
                let input =
                    buffered_assembly::build_first_frame_input(&header, frame_bytes, routing)
                        .map_err(|err| record_invalid(&mut failures, correlation_id, err))?;
                stream_assembly::accept_streaming_first(state, &mut failures, correlation_id, input)
            } else {
                let mut context = buffered_assembly::AssemblyContext {
                    state,
                    failures: &mut failures,
                    correlation_id,
                };
                let Some(msg) = buffered_assembly::admit_first_frame(
                    &mut context,
                    &header,
                    frame_bytes,
                    routing,
                )?
                else {
                    return Ok(FrameOutcome::Pending);
                };
                Ok(buffered_outcome(msg, streaming_routes))
            }
        }
        FrameHeader::Continuation(header) => {
            let key = header.message_key;
            if state.is_streaming_key(key) {
                if frame_bytes.len() != header.body_len {
                    return Ok(record_length_mismatch(
                        &mut failures,
                        correlation_id,
                        header.body_len,
                        frame_bytes.len(),
                    ));
                }
                stream_assembly::accept_streaming_continuation(
                    state,
                    &mut failures,
                    correlation_id,
                    key,
                    frame_bytes,
                    &header,
                )
            } else {
                let mut context = buffered_assembly::AssemblyContext {
                    state,
                    failures: &mut failures,
                    correlation_id,
                };
                let Some(msg) = buffered_assembly::admit_continuation_frame(
                    &mut context,
                    &header,
                    frame_bytes,
                )?
                else {
                    return Ok(FrameOutcome::Pending);
                };
                Ok(buffered_outcome(msg, streaming_routes))
            }
        }
    }
}

/// Build a dispatch outcome when no assembler or state is available.
fn fallback_outcome(
    env: Envelope,
    streaming_routes: &HashMap<u32, StreamingHandler>,
    stream_live: bool,
) -> FrameOutcome {
    if !streaming_routes.contains_key(&env.id) {
        return FrameOutcome::Buffered(env);
    }
    let parts = RequestParts::new(env.id, env.correlation_id, vec![]);
    if stream_live {
        return FrameOutcome::StreamFallback {
            parts,
            body: Bytes::from(env.payload),
        };
    }
    FrameOutcome::StreamStarted {
        key: None,
        parts,
        chunk: Bytes::from(env.payload),
        is_last: true,
    }
}

/// Convert a completed buffered assembly into its dispatch outcome.
fn buffered_outcome(
    msg: AssembledMessage,
    streaming_routes: &HashMap<u32, StreamingHandler>,
) -> FrameOutcome {
    let route_id: u32 = msg.routing().envelope_id.into();
    if streaming_routes.contains_key(&route_id) {
        let parts = request_parts(msg.routing(), msg.metadata().to_vec());
        return FrameOutcome::StreamFallback {
            parts,
            body: Bytes::from(msg.into_body()),
        };
    }
    FrameOutcome::Buffered(Envelope::from_assembled(&msg))
}

/// Build request parts from assembly routing and metadata.
fn request_parts(routing: EnvelopeRouting, metadata: Vec<u8>) -> RequestParts {
    RequestParts::new(
        routing.envelope_id.into(),
        routing.correlation_id.map(Into::into),
        metadata,
    )
}

/// Record a malformed frame and return the resulting error for `?`.
fn record_invalid(
    failures: &mut DeserFailureTracker<'_>,
    correlation_id: Option<u64>,
    err: io::Error,
) -> io::Error {
    let _ = failures.record(
        correlation_id,
        "message assembly first frame malformed",
        &err,
    );
    err
}

/// Record a continuation length mismatch and return a pending outcome.
fn record_length_mismatch(
    failures: &mut DeserFailureTracker<'_>,
    correlation_id: Option<u64>,
    expected: usize,
    actual: usize,
) -> FrameOutcome {
    let _ = failures.record(
        correlation_id,
        "message assembly continuation frame length mismatch",
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "message assembly continuation length mismatch: expected {expected}, got {actual}"
            ),
        ),
    );
    FrameOutcome::Pending
}
