//! Turning decoder failures into what the read and dispatch loops do next.
//!
//! [`RecoveryCoordinator`] decides one failure at a time; the helpers here
//! carry that decision through the loop that owns the connection. Reading has
//! to know whether a failure was discarded or ends the stream, and dispatching
//! has to report a failure buffered behind a response that closed the
//! connection.

use std::io;

use tokio_util::sync::CancellationToken;

use super::{super::frames::InboundFrames, RecoveryCoordinator, RecoveryState};
use crate::{
    app::{
        frame_handling::Dispatch,
        recovery_codec::{DecodeFailure, DecodeOrigin, DecodedItem},
    },
    codec::{CodecError, eof_condition},
};

/// Resolve one read item into the frame the loop must dispatch.
///
/// A decoder failure is handled here rather than inside the dispatch step: the
/// recovery policy can discard or quarantine it, which is a property of the
/// read loop rather than of dispatching one decoded frame. `None` means the
/// applied policy discarded the failure, so the loop reads again; a policy that
/// ends the connection returns the decoder's own error.
pub(in crate::app::inbound_handler) async fn resolve_read_item<T>(
    item: DecodedItem<T>,
    inbound: &mut InboundFrames<DecodedItem<T>, T>,
    recovery: &mut RecoveryCoordinator<'_>,
    shutdown: &CancellationToken,
) -> io::Result<Option<T>> {
    match item {
        DecodedItem::Frame(frame) => {
            recovery.record_decoded_frame();
            Ok(Some(frame))
        }
        DecodedItem::Failure(failure) => recovery
            .apply(
                failure,
                RecoveryState {
                    pipeline: &mut inbound.pipeline,
                    message_assembly: &mut inbound.message_assembly,
                    shutdown,
                },
            )
            .await
            .map_or(Ok(None), Err),
    }
}

/// Keep an item read while another step is pending, or end the connection.
///
/// A `decode_eof` failure is terminal by definition: the peer has stopped
/// sending, so no later frame can arrive and no selected policy can keep the
/// connection useful. Returning the decoder's own error ends the connection
/// here, which is what lets a pending response or a stalled push write observe
/// a truncated peer at all; the failure is reported first, as the disconnect it
/// is, so `on_error` and the counter still see it exactly once.
pub(in crate::app::inbound_handler) fn hold_or_end<T>(
    item: DecodedItem<T>,
    recovery: &RecoveryCoordinator<'_>,
) -> io::Result<DecodedItem<T>> {
    match item {
        DecodedItem::Failure(failure) if failure.origin() == DecodeOrigin::DecodeEof => {
            // `report_terminal` consumes the failure, and `CodecError` is not
            // `Clone`, so the error the caller receives is rebuilt from the
            // copyable EOF condition the original carries. Everything else
            // about a decoder error is terminal for this connection anyway.
            let (error, origin, consumed) = failure.into_parts();
            let rebuilt = match eof_condition(&error) {
                Some(eof) => CodecError::Eof(eof).into(),
                None => io::Error::new(error.kind(), error.to_string()),
            };
            recovery.report_terminal(DecodeFailure::new(error, origin, consumed));
            Err(rebuilt)
        }
        other => Ok(other),
    }
}

/// Report a decoder failure buffered when the connection stops reading.
///
/// The read half holds at most one item while a response runs. Three exits can
/// leave that item unread: a response that ends the connection, a shutdown
/// that cancels the next read before the buffer is drained, and memory
/// pressure that ends the connection before the next read. None reaches
/// [`resolve_read_item`], so its hook callback and metric would be lost.
/// Nothing is applied here: the connection is already ending, so only the
/// observation is owed, and it is reported as a disconnect because no selected
/// policy can take effect on a closing connection.
pub(in crate::app::inbound_handler) fn report_pending_failure<T>(
    inbound: &mut InboundFrames<DecodedItem<T>, T>,
    recovery: &RecoveryCoordinator<'_>,
) {
    if let Some(DecodedItem::Failure(failure)) = inbound.pending.take() {
        recovery.report_terminal(failure);
    }
}

/// Settle one dispatch, reporting any buffered failure on a terminal outcome.
///
/// Continuing dispatch leaves a buffered failure for [`resolve_read_item`] on
/// the next iteration, which is where its policy is applied. An error or a
/// `Stop` ends the connection, so the pending failure is reported here instead,
/// as a disconnect: the hook callback and metric still observe it, but no
/// quarantine or further read is started for a connection that is closing.
pub(in crate::app::inbound_handler) fn settle_dispatch<T>(
    dispatch: io::Result<Dispatch>,
    inbound: &mut InboundFrames<DecodedItem<T>, T>,
    recovery: &RecoveryCoordinator<'_>,
) -> io::Result<Dispatch> {
    if matches!(dispatch, Ok(Dispatch::Continue)) {
        return Ok(Dispatch::Continue);
    }
    report_pending_failure(inbound, recovery);
    dispatch
}
