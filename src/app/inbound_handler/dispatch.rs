//! Decoding one inbound frame's payload into an [`Envelope`].
//!
//! The frame loop hands each raw frame to
//! [`core::decode_and_assemble`](super::core::decode_and_assemble), which decodes its
//! envelope here, then reassembles and assembles it. Decoding is separated
//! from those later stages because it is the only one that depends on the
//! serializer's metadata layout rather than on connection-local buffering.

use super::super::envelope::Envelope;
use crate::{
    frame::FrameMetadata,
    message::{DecodeWith, DeserializeContext},
    serializer::Serializer,
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
    parse_envelope(serializer, payload)
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
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "trailing bytes after envelope: frame must contain exactly one value",
        )
        .into());
    }
    // The envelope owns a freshly allocated payload vector, which is the
    // inbound copy. Recording from the decoded payload's length measures the
    // bytes the message holds, not the frame's metadata, and it sits after the
    // trailing-bytes check so a rejected frame is not counted as decoded.
    crate::benchmark_counters::record_deserialized(envelope.payload_bytes().len());
    Ok((envelope, consumed))
}

/// Detect a bincode budget failure so fallback decoding does not repeat the work.
fn is_bincode_limit_exceeded(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<bincode::error::DecodeError>()
        .is_some_and(|decode_error| {
            matches!(decode_error, bincode::error::DecodeError::LimitExceeded)
        })
}
