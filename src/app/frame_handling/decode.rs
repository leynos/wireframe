//! Helpers for decoding envelopes from inbound frames.

use std::io;

use super::DeserFailureTracker;
use crate::{app::Envelope, codec::FrameCodec};

/// Decode an envelope and apply connection-level deserialization failure policy.
pub(crate) fn decode_envelope<F>(
    parse_result: Result<(Envelope, usize), Box<dyn std::error::Error + Send + Sync>>,
    frame: &F::Frame,
    failures: &mut DeserFailureTracker<'_>,
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
            let correlation_id = F::correlation_id(frame);
            failures.record(correlation_id, "failed to decode message", err)?;
            Ok(None)
        }
    }
}
