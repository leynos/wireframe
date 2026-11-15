//! Buffered message assembly branches for routes that need the whole body.
//!
//! A buffered route receives one [`Envelope`] after every frame of a logical
//! message has arrived. These helpers validate frame layout, feed accepted
//! frames into connection-local assembly state, and convert a completed
//! message into the envelope the middleware chain expects.

use std::io;

use super::core::DeserFailureTracker;
use crate::{
    app::Envelope,
    message_assembler::{
        AssembledMessage,
        ContinuationFrameHeader,
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        MessageAssemblyState,
    },
};

/// Mutable assembly state plus the failure budget for one inbound frame.
pub(super) struct AssemblyContext<'a, 'b> {
    /// Connection-local assembler state updated by accepted fragments.
    pub(super) state: &'a mut MessageAssemblyState,
    /// Shared, connection-wide malformed-frame accounting borrowed from the
    /// loop while this inbound frame is processed.
    pub(super) failures: &'a mut DeserFailureTracker<'b>,
    /// Correlation metadata used to diagnose malformed frame errors.
    pub(super) correlation_id: Option<u64>,
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

/// Validate and feed a first assembly frame into connection-local state.
pub(super) fn admit_first_frame(
    context: &mut AssemblyContext<'_, '_>,
    header: &FirstFrameHeader,
    frame_bytes: &[u8],
    routing: EnvelopeRouting,
) -> io::Result<Option<AssembledMessage>> {
    let input = match build_first_frame_input(header, frame_bytes, routing) {
        Ok(input) => input,
        Err(err) => {
            return context.fail_invalid_none("message assembly first frame malformed", err);
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

/// Validate the frame layout and construct a [`FirstFrameInput`].
pub(super) fn build_first_frame_input<'a>(
    header: &'a FirstFrameHeader,
    frame_bytes: &'a [u8],
    routing: EnvelopeRouting,
) -> io::Result<FirstFrameInput<'a>> {
    let Some(expected_len) = header.metadata_len.checked_add(header.body_len) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message assembly first-frame declared length overflow",
        ));
    };

    if frame_bytes.len() != expected_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "message assembly first-frame length mismatch: expected {expected_len}, got {}",
                frame_bytes.len()
            ),
        ));
    }

    let Some((metadata, body)) = frame_bytes.split_at_checked(header.metadata_len) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message assembly first-frame metadata split failed",
        ));
    };

    FirstFrameInput::new(header, routing, metadata.to_vec(), body)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// Validate and append a continuation frame in sequence order.
pub(super) fn admit_continuation_frame(
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
    pub(super) fn from_assembled(assembled: &AssembledMessage) -> Self {
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
