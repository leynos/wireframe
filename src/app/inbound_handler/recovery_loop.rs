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
use crate::app::{frame_handling::Dispatch, recovery_codec::DecodedItem};

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

/// Report a decoder failure buffered when the connection stops reading.
///
/// The read half holds at most one item while a response runs. Two exits can
/// leave that item unread: a response that ends the connection, and a shutdown
/// that cancels the next read before the buffer is drained. Neither reaches
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
