//! Helpers for decoding envelopes from inbound frames.

use std::io;

use crate::{app::Envelope, codec::FrameCodec};

/// Decode an envelope and apply connection-level deserialization failure policy.
pub(crate) fn decode_envelope<F>(
    parse_result: Result<(Envelope, usize), Box<dyn std::error::Error + Send + Sync>>,
    frame: &F::Frame,
    deser_failures: &mut u32,
    max_deser_failures: u32,
) -> io::Result<Option<Envelope>>
where
    F: FrameCodec,
{
    match parse_result {
        Ok((mut env, _)) => {
            if env.correlation_id.is_none() {
                env.correlation_id = F::correlation_id(frame);
            }
            Ok(Some(env))
        }
        Err(err) => {
            *deser_failures += 1;
            let correlation_id = F::correlation_id(frame);
            let context = "failed to decode message";
            log::warn!("{context}: correlation_id={correlation_id:?}, error={err:?}");
            crate::metrics::inc_deser_errors();
            if *deser_failures >= max_deser_failures {
                log::warn!(
                    "closing connection after {deser_failures} deserialization failures: {context}"
                );
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "too many deserialization failures",
                ));
            }
            Ok(None)
        }
    }
}
