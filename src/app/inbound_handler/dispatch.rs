//! Decoding one inbound frame's payload into an [`Envelope`].
//!
//! The frame loop hands each raw frame to
//! [`core::decode_outcome`](super::core::decode_outcome), which decodes its
//! envelope here, then reassembles and assembles it. Decoding is separated
//! from those later stages because it is the only one that depends on the
//! serializer's metadata layout rather than on connection-local buffering.

use super::super::envelope::Envelope;
use crate::{
    frame::FrameMetadata,
    message::{DecodeWith, DeserializeContext},
    serializer::Serializer,
};

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
