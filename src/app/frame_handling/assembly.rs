//! Helpers for protocol-level message assembly on the inbound path.

use std::{io, num::NonZeroUsize, sync::Arc, time::Duration};

use log::debug;

use super::core::DeserFailureTracker;
use crate::{
    app::{Envelope, builder_defaults::default_fragmentation},
    codec::clamp_frame_length,
    fragment::FragmentationConfig,
    message_assembler::{
        AssembledMessage,
        ContinuationFrameHeader,
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        FrameHeader,
        MessageAssembler,
        MessageAssemblyState,
    },
};

/// Default timeout used when no fragmentation-derived timeout is available.
const DEFAULT_MESSAGE_ASSEMBLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Borrowed inbound runtime state used for message assembly.
pub(crate) struct AssemblyRuntime<'a> {
    pub(crate) assembler: Option<&'a Arc<dyn MessageAssembler>>,
    pub(crate) state: &'a mut Option<MessageAssemblyState>,
}

impl<'a> AssemblyRuntime<'a> {
    /// Create assembly runtime accessors for one inbound frame.
    #[must_use]
    pub(crate) fn new(
        assembler: Option<&'a Arc<dyn MessageAssembler>>,
        state: &'a mut Option<MessageAssemblyState>,
    ) -> Self {
        Self { assembler, state }
    }
}

/// Build a connection-scoped message assembly state from known budgets.
#[must_use]
pub(crate) fn new_message_assembly_state(
    fragmentation: Option<FragmentationConfig>,
    frame_budget: usize,
) -> MessageAssemblyState {
    let config = fragmentation.or_else(|| default_fragmentation(frame_budget));
    let max_message_size = config.map_or_else(
        || NonZeroUsize::new(clamp_frame_length(frame_budget)).unwrap_or(NonZeroUsize::MIN),
        |cfg| cfg.max_message_size,
    );
    let timeout = config.map_or(DEFAULT_MESSAGE_ASSEMBLY_TIMEOUT, |cfg| {
        cfg.reassembly_timeout
    });

    MessageAssemblyState::new(max_message_size, timeout)
}

/// Purge stale in-flight assemblies.
pub(crate) fn purge_expired_assemblies(assembly: &mut Option<MessageAssemblyState>) {
    let Some(state) = assembly.as_mut() else {
        return;
    };

    let evicted = state.purge_expired();
    if !evicted.is_empty() {
        debug!(
            "purged expired message assemblies: count={}, keys={evicted:?}",
            evicted.len()
        );
    }
}

/// Apply protocol-level message assembly to a complete post-fragment envelope.
pub(crate) fn assemble_if_needed(
    runtime: AssemblyRuntime<'_>,
    deser_failures: &mut u32,
    env: Envelope,
    max_deser_failures: u32,
) -> io::Result<Option<Envelope>> {
    let AssemblyRuntime {
        assembler,
        state: assembly,
    } = runtime;
    let Some(assembler) = assembler else {
        return Ok(Some(env));
    };
    let Some(state) = assembly.as_mut() else {
        return Ok(Some(env));
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
            return Ok(None);
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
        return Ok(None);
    };

    let mut context = AssemblyContext {
        state,
        failures: &mut failures,
        correlation_id,
    };

    let routing = EnvelopeRouting {
        envelope_id: env.id.into(),
        correlation_id: env.correlation_id.map(Into::into),
    };

    match parsed.into_header() {
        FrameHeader::First(header) => {
            let Some(result) = process_first_frame(&mut context, &header, frame_bytes, routing)?
            else {
                return Ok(None);
            };
            Ok(Some(Envelope::from_assembled(&result)))
        }
        FrameHeader::Continuation(header) => {
            let result = process_continuation_frame(&mut context, &header, frame_bytes)?;
            Ok(result.map(|assembled| Envelope::from_assembled(&assembled)))
        }
    }
}

struct AssemblyContext<'a, 'b> {
    state: &'a mut MessageAssemblyState,
    failures: &'a mut DeserFailureTracker<'b>,
    correlation_id: Option<u64>,
}

impl AssemblyContext<'_, '_> {
    /// Record a failure and return `Ok(None)` to continue processing.
    fn fail_invalid_none(
        &mut self,
        context: &str,
        err: impl std::fmt::Debug,
    ) -> io::Result<Option<AssembledMessage>> {
        self.failures.record(self.correlation_id, context, err)?;
        Ok(None)
    }
}

fn process_first_frame(
    context: &mut AssemblyContext<'_, '_>,
    header: &FirstFrameHeader,
    frame_bytes: &[u8],
    routing: EnvelopeRouting,
) -> io::Result<Option<AssembledMessage>> {
    let Some(expected_len) = header.metadata_len.checked_add(header.body_len) else {
        return context.fail_invalid_none(
            "message assembly first frame length overflow",
            io::Error::new(
                io::ErrorKind::InvalidData,
                "message assembly first-frame declared length overflow",
            ),
        );
    };

    if frame_bytes.len() != expected_len {
        return context.fail_invalid_none(
            "message assembly first frame length mismatch",
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "message assembly first-frame length mismatch: expected {expected_len}, got {}",
                    frame_bytes.len()
                ),
            ),
        );
    }

    let Some((metadata, body)) = frame_bytes.split_at_checked(header.metadata_len) else {
        return context.fail_invalid_none(
            "message assembly first frame metadata split failed",
            io::Error::new(
                io::ErrorKind::InvalidData,
                "message assembly first-frame metadata split failed",
            ),
        );
    };

    let input = match FirstFrameInput::new(header, routing, metadata.to_vec(), body) {
        Ok(input) => input,
        Err(err) => {
            return context.fail_invalid_none(
                "message assembly first frame input validation failed",
                io::Error::new(io::ErrorKind::InvalidData, err),
            );
        }
    };

    match context.state.accept_first_frame(input) {
        Ok(result) => Ok(result),
        Err(err) => context.fail_invalid_none(
            "message assembly first frame rejected",
            io::Error::new(io::ErrorKind::InvalidData, err),
        ),
    }
}

fn process_continuation_frame(
    context: &mut AssemblyContext<'_, '_>,
    header: &ContinuationFrameHeader,
    frame_bytes: &[u8],
) -> io::Result<Option<AssembledMessage>> {
    if frame_bytes.len() != header.body_len {
        return context.fail_invalid_none(
            "message assembly continuation frame length mismatch",
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "message assembly continuation length mismatch: expected {}, got {}",
                    header.body_len,
                    frame_bytes.len()
                ),
            ),
        );
    }

    match context.state.accept_continuation_frame(header, frame_bytes) {
        Ok(result) => Ok(result),
        Err(err) => context.fail_invalid_none(
            "message assembly continuation frame rejected",
            io::Error::new(io::ErrorKind::InvalidData, err),
        ),
    }
}

impl Envelope {
    /// Construct an envelope from a completed message assembly result.
    ///
    /// Uses the [`EnvelopeRouting`] stored in the assembled message, which
    /// originates from the first frame.
    fn from_assembled(assembled: &AssembledMessage) -> Self {
        let routing = assembled.routing();
        let metadata = assembled.metadata();
        let body = assembled.body();
        let mut payload = Vec::with_capacity(metadata.len().saturating_add(body.len()));
        payload.extend_from_slice(metadata);
        payload.extend_from_slice(body);

        Self::new(
            routing.envelope_id.into(),
            routing.correlation_id.map(Into::into),
            payload,
        )
    }
}
