//! Recovery-policy evaluation for decoder failures on a live connection.
//!
//! Each connection owns one [`RecoveryCoordinator`]. It converts a captured
//! [`DecodeFailure`] into an action for the read loop, consulting the installed
//! [`RecoveryPolicyHook`] when there is one. The coordinator is also the single
//! place that reports a failure to the hook's `on_error` callback, emits
//! `wireframe_codec_errors_total`, and logs the selected and applied policies.

use std::{io, net::SocketAddr, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::{
    super::{
        frame_handling::Dispatch,
        recovery_codec::{DecodeFailure, DecodeOrigin, DecodedItem},
    },
    frames::{InboundFrames, purge_expired},
};
use crate::{
    app::codec_driver::FramePipeline,
    codec::{CodecError, CodecErrorContext, RecoveryConfig, RecoveryPolicy, RecoveryPolicyHook},
    message_assembler::MessageAssemblyState,
};

/// What the connection loop should do after a decoder failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecoveryAction {
    /// Discard the malformed frame and keep reading.
    Drop,
    /// Pause the connection for this duration, then keep reading.
    Quarantine(Duration),
    /// Stop reading and return the original error to the teardown path.
    Disconnect,
}

/// The applied action paired with the error that produced it.
pub(super) struct RecoveryOutcome {
    /// Action the read loop must perform.
    pub(super) action: RecoveryAction,
    /// Original decoder error, returned unchanged when disconnecting.
    pub(super) error: io::Error,
}

/// Connection state a quarantine pause must observe and purge.
///
/// The pause belongs to the read loop, so the loop owns this state and the
/// coordinator only borrows it while applying one decision.
pub(super) struct RecoveryState<'a, T> {
    /// Fragmentation pipeline whose expired buffers the pause drops.
    pub(super) pipeline: &'a mut FramePipeline<T>,
    /// Assembly state whose expired assemblies the pause drops.
    pub(super) message_assembly: &'a mut Option<MessageAssemblyState>,
    /// Connection shutdown signal; cancelling it ends the pause early.
    pub(super) shutdown: &'a CancellationToken,
}

/// Per-connection recovery state: the installed hook and drop accounting.
pub(super) struct RecoveryCoordinator<'a> {
    /// Hook consulted for each failure, or `None` to preserve terminal errors.
    hook: Option<&'a dyn RecoveryPolicyHook>,
    /// Global settings such as the consecutive-drop limit.
    config: RecoveryConfig,
    /// Failures dropped since the last successfully decoded frame.
    consecutive_drops: u32,
    /// Peer address reported to the hook, when the transport exposed one.
    peer_address: Option<SocketAddr>,
}

impl<'a> RecoveryCoordinator<'a> {
    /// Create the coordinator for one connection.
    pub(super) const fn new(
        hook: Option<&'a dyn RecoveryPolicyHook>,
        config: RecoveryConfig,
    ) -> Self {
        Self {
            hook,
            config,
            consecutive_drops: 0,
            peer_address: None,
        }
    }

    /// Attach the peer address the connection was accepted from.
    ///
    /// Callers that wrap a bare byte stream, such as the compatibility
    /// connection path, leave it unset; the hook then sees an empty address.
    pub(super) const fn with_peer_address(mut self, address: Option<SocketAddr>) -> Self {
        self.peer_address = address;
        self
    }

    /// Record that a real frame decoded, restarting the drop streak.
    pub(super) const fn record_decoded_frame(&mut self) { self.consecutive_drops = 0; }

    /// Decide what to do with one decoder failure and report it.
    ///
    /// The original error is always returned so a disconnect can propagate it
    /// without rebuilding it from text. `on_error` observes the *applied*
    /// policy, which may differ from the hook's selection after a fallback.
    pub(super) fn evaluate(&mut self, failure: DecodeFailure) -> RecoveryOutcome {
        let (error, origin, consumed) = failure.into_parts();

        // A decoder that already wraps a `CodecError` carries a precise
        // classification. Anything else is presented as an I/O error so hooks
        // still see a typed error, then the exact original error is recovered
        // for the return path.
        if let Some(codec_error) = codec_error_in(&error) {
            let action = self.decide(codec_error, origin, consumed);
            return RecoveryOutcome { action, error };
        }

        let codec_error = CodecError::Io(error);
        let action = self.decide(&codec_error, origin, consumed);
        let original = match codec_error {
            CodecError::Io(io_error) => io_error,
            other => other.into(),
        };
        RecoveryOutcome {
            action,
            error: original,
        }
    }

    /// Decide on one failure and carry out the resulting action.
    ///
    /// Yields the decoder's original error when the connection must end. The
    /// coordinator has already forced a disconnect for a policy that would
    /// discard bytes the decoder did not consume, so a drop here always
    /// advances the read position.
    ///
    /// A quarantine pause is cut short when the connection's shutdown token is
    /// cancelled, so a hook-chosen pause cannot delay a graceful drain; the
    /// loop observes the same token again at its next read.
    pub(super) async fn apply<T>(
        &mut self,
        failure: DecodeFailure,
        state: RecoveryState<'_, T>,
    ) -> Option<io::Error> {
        let RecoveryOutcome { action, error } = self.evaluate(failure);
        let RecoveryState {
            pipeline,
            message_assembly,
            shutdown,
        } = state;
        match action {
            RecoveryAction::Drop => None,
            RecoveryAction::Quarantine(duration) => {
                await_quarantine(duration, shutdown).await;
                purge_expired(pipeline, message_assembly);
                None
            }
            // Return the decoder's own error so the caller's teardown path sees
            // the value it produced rather than one rebuilt from display text.
            RecoveryAction::Disconnect => Some(error),
        }
    }

    /// Apply the documented decision order to one classified failure.
    fn decide(
        &mut self,
        error: &CodecError,
        origin: DecodeOrigin,
        consumed: usize,
    ) -> RecoveryAction {
        let context = self.error_context();
        let selected = self.select_policy(error, &context);
        let applied = self.apply_fallbacks(selected, origin, consumed);
        let action = self.action_for(applied, error, &context);

        if let Some(hook) = self.hook {
            hook.on_error(error, &context, applied);
        }
        crate::metrics::inc_codec_error(error.error_type(), applied.as_str());
        self.log_failure(error, selected, applied);

        action
    }

    /// Build the context reported to the hook for one failure.
    ///
    /// The connection loop knows only the peer address; the remaining fields
    /// describe a specific frame, which does not exist yet when decoding has
    /// failed.
    fn error_context(&self) -> CodecErrorContext {
        self.peer_address
            .map_or_else(CodecErrorContext::new, |address| {
                CodecErrorContext::new().with_peer_address(address)
            })
    }

    /// Ask the hook for a policy, or disconnect when no hook is installed.
    fn select_policy(&self, error: &CodecError, context: &CodecErrorContext) -> RecoveryPolicy {
        self.hook.map_or(RecoveryPolicy::Disconnect, |hook| {
            hook.recovery_policy(error, context)
        })
    }

    /// Apply the safety fallbacks that override the hook's selection.
    ///
    /// `decode_eof` means the peer stopped sending, and a failure that consumed
    /// no bytes would repeat forever. Both force a disconnect. A drop streak
    /// beyond the configured maximum escalates in the same way.
    fn apply_fallbacks(
        &mut self,
        selected: RecoveryPolicy,
        origin: DecodeOrigin,
        consumed: usize,
    ) -> RecoveryPolicy {
        if origin == DecodeOrigin::DecodeEof {
            return RecoveryPolicy::Disconnect;
        }
        if consumed == 0 && matches!(selected, RecoveryPolicy::Drop | RecoveryPolicy::Quarantine) {
            return RecoveryPolicy::Disconnect;
        }
        if selected != RecoveryPolicy::Drop {
            return selected;
        }

        self.consecutive_drops = self.consecutive_drops.saturating_add(1);
        if self.consecutive_drops > self.config.max_consecutive_drops {
            RecoveryPolicy::Disconnect
        } else {
            RecoveryPolicy::Drop
        }
    }

    /// Translate an applied policy into a loop action.
    fn action_for(
        &self,
        applied: RecoveryPolicy,
        error: &CodecError,
        context: &CodecErrorContext,
    ) -> RecoveryAction {
        match applied {
            RecoveryPolicy::Drop => RecoveryAction::Drop,
            RecoveryPolicy::Disconnect => RecoveryAction::Disconnect,
            // A quarantine is only reachable with a hook installed, because
            // `select_policy` forces disconnect otherwise. The hook controls
            // the pause; `RecoveryConfig::quarantine_duration` is not consulted.
            RecoveryPolicy::Quarantine => RecoveryAction::Quarantine(
                self.hook.map_or(self.config.quarantine_duration, |hook| {
                    hook.quarantine_duration(error, context)
                }),
            ),
        }
    }

    /// Log the failure, honouring the dropped-frame logging switch.
    ///
    /// The policy fields are structured so a subscriber can filter on them
    /// rather than parsing the message text.
    fn log_failure(&self, error: &CodecError, selected: RecoveryPolicy, applied: RecoveryPolicy) {
        let should_log = applied != RecoveryPolicy::Drop || self.config.log_dropped_frames;
        if should_log {
            warn!(
                error_type = error.error_type(),
                selected_policy = selected.as_str(),
                applied_policy = applied.as_str(),
                "codec error recovery"
            );
        }
    }
}

/// Wait out a quarantine pause, or stop waiting when the connection shuts down.
async fn await_quarantine(duration: Duration, shutdown: &CancellationToken) {
    tokio::select! {
        () = tokio::time::sleep(duration) => {}
        () = shutdown.cancelled() => {}
    }
}

/// Borrow the `CodecError` a decoder attached to its `io::Error`, if any.
fn codec_error_in(error: &io::Error) -> Option<&CodecError> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<CodecError>())
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

/// Resolve one read item into the frame the loop must dispatch.
///
/// A decoder failure is handled here rather than inside the dispatch step: the
/// recovery policy can discard or quarantine it, which is a property of the
/// read loop rather than of dispatching one decoded frame. `None` means the
/// applied policy discarded the failure, so the loop reads again; a policy that
/// ends the connection returns the decoder's own error.
pub(super) async fn resolve_read_item<T>(
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

/// Report a decoder failure prefetched while a response was in flight.
///
/// The read half buffers at most one item while a response runs. When that
/// response ends the connection, the buffered failure never reaches
/// [`resolve_read_item`], so its hook callback and metric would be lost.
/// Nothing is applied here: the connection is already ending, so only the
/// observation is owed.
pub(super) fn report_pending_failure<T>(
    inbound: &mut InboundFrames<DecodedItem<T>, T>,
    recovery: &mut RecoveryCoordinator<'_>,
) {
    if let Some(DecodedItem::Failure(failure)) = inbound.pending.take() {
        drop(recovery.evaluate(failure));
    }
}

/// Settle one dispatch, reporting any buffered failure on a terminal outcome.
///
/// Continuing dispatch leaves a buffered failure for
/// [`resolve_read_item`] on the next iteration, which is where its policy is
/// applied. An error or a `Stop` ends the connection, so the pending failure is
/// reported here instead: the hook callback and metric still observe it, but no
/// quarantine or further read is started for a connection that is closing.
pub(super) fn settle_dispatch<T>(
    dispatch: io::Result<Dispatch>,
    inbound: &mut InboundFrames<DecodedItem<T>, T>,
    recovery: &mut RecoveryCoordinator<'_>,
) -> io::Result<Dispatch> {
    if matches!(dispatch, Ok(Dispatch::Continue)) {
        return Ok(Dispatch::Continue);
    }
    report_pending_failure(inbound, recovery);
    dispatch
}
