//! Streaming branches of protocol-level message assembly.
//!
//! These helpers translate validated streaming-assembly outcomes into the
//! frame outcomes consumed by the connection loop. They never buffer body
//! bytes: each chunk is copied into a [`Bytes`] value and handed to the
//! dispatch slot, which admits it to the request-body channel.

use std::io;

use bytes::Bytes;

use super::assembly::FrameOutcome;
use crate::{
    app::frame_handling::DeserFailureTracker,
    message_assembler::{
        ContinuationFrameHeader,
        FirstFrameInput,
        MessageAssemblyState,
        MessageKey,
        StreamingContinuation,
        StreamingFailure,
    },
    request::RequestParts,
};

/// Admit a first frame for a streaming route and build its dispatch outcome.
///
/// # Errors
///
/// Returns an error only when recording a rejected frame exhausts the
/// malformed-frame budget. Assembly rejections are reported through the
/// tracker and yield [`FrameOutcome::Pending`].
pub(super) fn accept_streaming_first(
    state: &mut MessageAssemblyState,
    failures: &mut DeserFailureTracker<'_>,
    correlation_id: Option<u64>,
    input: FirstFrameInput<'_>,
) -> io::Result<FrameOutcome> {
    match state.accept_streaming_first_frame(input) {
        Ok(first) => {
            let routing = first.routing();
            let key = first.key();
            let assembly = first.assembly_id();
            let chunk = Bytes::copy_from_slice(first.chunk());
            let is_last = first.is_last();
            let parts = RequestParts::new(
                routing.envelope_id.into(),
                routing.correlation_id.map(Into::into),
                first.into_metadata(),
            );
            Ok(FrameOutcome::StreamStarted {
                key: Some(key),
                assembly,
                parts,
                chunk,
                is_last,
            })
        }
        Err(error) => {
            failures.record(
                correlation_id,
                "streaming message assembly first frame rejected",
                &error,
            )?;
            Ok(FrameOutcome::Pending)
        }
    }
}

/// One continuation frame's body and the header that describes it.
#[derive(Debug)]
pub(super) struct ContinuationFrame<'a> {
    /// Body bytes carried by the frame.
    pub(super) body: &'a [u8],
    /// Validated continuation header for the frame.
    pub(super) header: &'a ContinuationFrameHeader,
}

/// Admit a continuation for an existing streaming message.
///
/// A terminal failure (the assembly was removed) yields
/// [`FrameOutcome::StreamFailed`] so the connection can fail the handler's
/// body. A recoverable continuity error is counted and yields
/// [`FrameOutcome::Pending`], leaving the assembly available for a
/// retransmission.
///
/// # Errors
///
/// Returns an error only when recording a rejected frame exhausts the
/// malformed-frame budget.
pub(super) fn accept_streaming_continuation(
    state: &mut MessageAssemblyState,
    failures: &mut DeserFailureTracker<'_>,
    correlation_id: Option<u64>,
    frame: &ContinuationFrame<'_>,
) -> io::Result<FrameOutcome> {
    let body = frame.body;
    let header = frame.header;
    match state.accept_streaming_continuation(header, body) {
        Ok(StreamingContinuation::Chunk {
            key,
            chunk,
            is_last,
        }) => Ok(FrameOutcome::StreamChunk {
            key,
            chunk: Bytes::copy_from_slice(chunk),
            is_last,
        }),
        Ok(StreamingContinuation::Discarded { .. }) => Ok(FrameOutcome::Discarded),
        Err(failure) => {
            failures.record(
                correlation_id,
                "streaming message assembly continuation rejected",
                &failure,
            )?;
            Ok(stream_failure_outcome(header.message_key, &failure))
        }
    }
}

/// Convert a terminal streaming failure into a connection-visible outcome.
fn stream_failure_outcome(key: MessageKey, failure: &StreamingFailure) -> FrameOutcome {
    if failure.is_terminal() {
        FrameOutcome::StreamFailed {
            key,
            error: io::Error::new(io::ErrorKind::InvalidData, failure.to_string()),
        }
    } else {
        FrameOutcome::Pending
    }
}
