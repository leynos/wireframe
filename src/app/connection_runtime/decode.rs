//! Envelope decoding, reassembly, and assembly for one inbound frame.

use std::sync::Arc;

use tokio::io;

use super::super::inbound_handler::MAX_DESER_FAILURES;
use crate::{
    app::{codec_driver::FramePipeline, envelope::Envelope, frame_handling},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, DeserializeContext},
    message_assembler::{MessageAssembler, MessageAssemblyState},
    serializer::Serializer,
};

/// State needed to turn a raw frame into a dispatchable envelope.
pub(super) struct DispatchBuildContext<'a, F>
where
    F: FrameCodec,
{
    /// Raw frame borrowed while decoding its envelope metadata and payload.
    pub(super) frame: &'a F::Frame,
    /// Pipeline needed to reassemble fragmented input and emit responses.
    pub(super) pipeline: &'a mut FramePipeline<F::Frame>,
    /// Mutable assembly state retained across inbound frames.
    pub(super) message_assembly: &'a mut Option<MessageAssemblyState>,
    /// Failure counter used to enforce the malformed-input limit.
    pub(super) deser_failures: &'a mut u32,
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
pub(crate) fn parse_envelope<S>(
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

/// Build a dispatchable envelope through decode, reassembly, and assembly.
///
/// Returns `None` when the frame was consumed without producing a complete
/// message, which keeps the connection open for a later fragment.
pub(super) fn build_dispatchable_envelope<S, F>(
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
