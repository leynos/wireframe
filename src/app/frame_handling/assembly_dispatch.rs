//! Routing of one assembled frame into a streaming or buffered outcome.
//!
//! [`dispatch_assembled`] is the seam between message assembly and connection
//! dispatch. It decides, per frame header, whether the frame continues a live
//! streaming body, opens one, or completes a buffered message. Keeping that
//! decision apart from the assembly bookkeeping in [`super::assembly`] leaves
//! each module with a single responsibility.

use std::{collections::HashMap, io};

use bytes::Bytes;

use super::{
    assembly::FrameOutcome,
    buffered_assembly,
    core::DeserFailureTracker,
    stream_assembly,
};
use crate::{
    app::{Envelope, StreamingHandler},
    message_assembler::{
        AssembledMessage,
        ContinuationFrameHeader,
        EnvelopeRouting,
        FirstFrameHeader,
        FrameHeader,
        MessageAssemblyState,
    },
    request::RequestParts,
};

/// One assembled frame's inputs, grouped for the header dispatch arms.
pub(super) struct AssembledFrame<'a, 'b, 'c> {
    /// Connection-local assembly state.
    pub(super) state: &'a mut MessageAssemblyState,
    /// Malformed-frame accounting for this connection.
    pub(super) failures: &'a mut DeserFailureTracker<'b>,
    /// Correlation identifier inherited from the envelope.
    pub(super) correlation_id: Option<u64>,
    /// Streaming handlers keyed by message identifier.
    pub(super) streaming_routes: &'c HashMap<u32, StreamingHandler>,
    /// Whether another streaming body owns the dispatch slot.
    pub(super) stream_live: bool,
    /// Envelope identifier from the transport frame.
    pub(super) envelope_id: u32,
    /// Body bytes that follow the frame header.
    pub(super) body: &'c [u8],
}

/// Route one assembled frame by header kind and target route.
pub(super) fn dispatch_assembled(
    header: FrameHeader,
    frame: AssembledFrame<'_, '_, '_>,
) -> io::Result<FrameOutcome> {
    match header {
        FrameHeader::First(first) => dispatch_first(&first, frame),
        FrameHeader::Continuation(continuation) => dispatch_continuation(&continuation, frame),
    }
}

/// Route a first frame, opening a streaming body when the route streams.
fn dispatch_first(
    header: &FirstFrameHeader,
    frame: AssembledFrame<'_, '_, '_>,
) -> io::Result<FrameOutcome> {
    let AssembledFrame {
        state,
        failures,
        correlation_id,
        streaming_routes,
        stream_live,
        envelope_id,
        body,
    } = frame;
    let routing = EnvelopeRouting {
        envelope_id: envelope_id.into(),
        correlation_id: correlation_id.map(Into::into),
    };
    if opens_stream(streaming_routes, envelope_id, stream_live) {
        return match buffered_assembly::build_first_frame_input(header, body, routing) {
            Ok(input) => {
                stream_assembly::accept_streaming_first(state, failures, correlation_id, input)
            }
            Err(err) => record_malformed_first(failures, correlation_id, err),
        };
    }
    let mut context = assembly_context(state, failures, correlation_id);
    let admitted = buffered_assembly::admit_first_frame(&mut context, header, body, routing)?;
    Ok(buffered_or_pending(admitted, streaming_routes))
}

/// Route a continuation frame to the live stream or to buffered assembly.
fn dispatch_continuation(
    header: &ContinuationFrameHeader,
    frame: AssembledFrame<'_, '_, '_>,
) -> io::Result<FrameOutcome> {
    let AssembledFrame {
        state,
        failures,
        correlation_id,
        streaming_routes,
        body,
        ..
    } = frame;
    let mut context = assembly_context(state, failures, correlation_id);
    if context.state.is_streaming_key(header.message_key) {
        return accept_stream_chunk(header, body, context);
    }
    let admitted = buffered_assembly::admit_continuation_frame(&mut context, header, body)?;
    Ok(buffered_or_pending(admitted, streaming_routes))
}

/// Hand one continuation to the live streaming body after a length check.
fn accept_stream_chunk(
    header: &ContinuationFrameHeader,
    body: &[u8],
    context: buffered_assembly::AssemblyContext<'_, '_>,
) -> io::Result<FrameOutcome> {
    let buffered_assembly::AssemblyContext {
        state,
        failures,
        correlation_id,
    } = context;
    if body.len() != header.body_len {
        return record_length_mismatch(failures, correlation_id, header.body_len, body.len());
    }
    let continuation = stream_assembly::ContinuationFrame { body, header };
    stream_assembly::accept_streaming_continuation(state, failures, correlation_id, &continuation)
}

/// Return whether a first frame should open a streaming body.
fn opens_stream(
    streaming_routes: &HashMap<u32, StreamingHandler>,
    envelope_id: u32,
    stream_live: bool,
) -> bool {
    streaming_routes.contains_key(&envelope_id) && !stream_live
}

/// Borrow one frame's assembly context.
const fn assembly_context<'a, 'b>(
    state: &'a mut MessageAssemblyState,
    failures: &'a mut DeserFailureTracker<'b>,
    correlation_id: Option<u64>,
) -> buffered_assembly::AssemblyContext<'a, 'b> {
    buffered_assembly::AssemblyContext {
        state,
        failures,
        correlation_id,
    }
}

/// Map a completed buffered assembly to its outcome, keeping incomplete ones pending.
fn buffered_or_pending(
    admitted: Option<AssembledMessage>,
    streaming_routes: &HashMap<u32, StreamingHandler>,
) -> FrameOutcome {
    admitted.map_or_else(
        || FrameOutcome::Pending,
        |msg| buffered_outcome(msg, streaming_routes),
    )
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

/// Record a malformed streaming first frame, staying pending below the limit.
///
/// A malformed frame is a peer error rather than a fatal one: the connection
/// ends only once the malformed-frame budget is exhausted. Returning pending
/// keeps that contract identical to the buffered path in
/// [`buffered_assembly::admit_first_frame`], so a streaming route does not
/// terminate a connection that a buffered route would have survived.
fn record_malformed_first(
    failures: &mut DeserFailureTracker<'_>,
    correlation_id: Option<u64>,
    err: io::Error,
) -> io::Result<FrameOutcome> {
    failures.record(
        correlation_id,
        "message assembly first frame malformed",
        err,
    )?;
    Ok(FrameOutcome::Pending)
}

/// Record a continuation length mismatch and return a pending outcome.
fn record_length_mismatch(
    failures: &mut DeserFailureTracker<'_>,
    correlation_id: Option<u64>,
    expected: usize,
    actual: usize,
) -> io::Result<FrameOutcome> {
    failures.record(
        correlation_id,
        "message assembly continuation frame length mismatch",
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "message assembly continuation length mismatch: expected {expected}, got {actual}"
            ),
        ),
    )?;
    Ok(FrameOutcome::Pending)
}
