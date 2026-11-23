//! Recovery-policy evaluation for decoder failures on a live connection.
//!
//! Each connection owns one [`RecoveryCoordinator`]. It converts a captured
//! [`DecodeFailure`] into an action for the read loop in separate steps:
//! [`RecoveryCoordinator::consider`] consults the installed
//! [`RecoveryPolicyHook`] and resolves the fallbacks,
//! [`RecoveryCoordinator::decide`] turns that choice into an action without
//! touching the hook, storing the drop streak records the decision, and
//! reporting sends the failure to the hook, the `wireframe_codec_errors_total`
//! metric, and the log.

use std::{io, net::SocketAddr, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::{
    super::recovery_codec::{DecodeFailure, DecodeOrigin},
    frames::purge_expired,
};
use crate::{
    app::codec_driver::FramePipeline,
    codec::{CodecError, CodecErrorContext, RecoveryConfig, RecoveryPolicy, RecoveryPolicyHook},
    message_assembler::MessageAssemblyState,
};

// Sibling files rather than a `recovery/` directory: this module remains one
// flat file beside its tests. The decision vocabulary lives apart from the
// coordinator so the pure step and the hook-consulting step stay separable.
#[path = "recovery_decision.rs"]
mod decision;
#[path = "recovery_loop.rs"]
mod recovery_loop;

pub(super) use decision::{
    Decision,
    RecoveryAction,
    ReportedPolicies,
    Selection,
    action_for,
    is_budgeted_drop,
};
pub(super) use recovery_loop::{
    hold_or_end,
    report_pending_failure,
    resolve_read_item,
    settle_dispatch,
};

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

    /// Decide one failure, then record and report that decision.
    ///
    /// [`Self::consider`] is the only hook consultation, and the decision it
    /// feeds is side-effect-free; this method performs the two commands that
    /// follow. The original error is always returned so a disconnect can
    /// propagate it without rebuilding it from text.
    pub(super) fn record_failure(&mut self, failure: DecodeFailure) -> RecoveryOutcome {
        let (error, origin, consumed) = failure.into_parts();

        // A decoder that already wraps a `CodecError` carries a precise
        // classification. Anything else is presented as an I/O error so hooks
        // still see a typed error, then the exact original error is recovered
        // for the return path.
        if let Some(codec_error) = codec_error_in(&error) {
            let decision = self.decide(self.consider(codec_error, origin, consumed));
            self.record_and_report(codec_error, decision);
            return RecoveryOutcome {
                action: decision.action,
                error,
            };
        }

        let codec_error = CodecError::Io(error);
        let decision = self.decide(self.consider(&codec_error, origin, consumed));
        self.record_and_report(&codec_error, decision);
        let original = match codec_error {
            CodecError::Io(io_error) => io_error,
            other => other.into(),
        };
        RecoveryOutcome {
            action: decision.action,
            error: original,
        }
    }

    /// Apply one failure's decision and carry out the resulting action.
    ///
    /// Yields the decoder's original error when the connection must end. A
    /// disconnect is already forced for a policy that would discard bytes the
    /// decoder did not consume, so a drop here advances the read position. A
    /// quarantine pause ends early when the connection shuts down.
    pub(super) async fn apply<T>(
        &mut self,
        failure: DecodeFailure,
        state: RecoveryState<'_, T>,
    ) -> Option<io::Error> {
        let RecoveryOutcome { action, error } = self.record_failure(failure);
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

    /// Report a failure the connection can no longer act on.
    ///
    /// A response that ends the connection abandons any prefetched failure, so
    /// the frame is discarded and the connection closes whatever the hook would
    /// have selected. Reporting that selection would claim a drop or a
    /// quarantine that never happened, so the failure is reported as the
    /// disconnect it actually is. The hook is not consulted.
    pub(super) fn report_terminal(&self, failure: DecodeFailure) -> io::Error {
        let (error, ..) = failure.into_parts();
        let policies = ReportedPolicies::terminal();

        match codec_error_in(&error) {
            Some(codec_error) => self.publish(codec_error, policies),
            // `publish` reports an owned `CodecError`, so a plain I/O error is
            // described for the callback and counter while the decoder's own
            // error is returned unchanged, keeping its kind and source chain.
            None => {
                let described = CodecError::Io(io::Error::new(error.kind(), error.to_string()));
                self.publish(&described, policies);
            }
        }
        error
    }

    /// Ask the hook what to do with one classified failure.
    ///
    /// The only step that consults the hook for a policy or a pause. Its result
    /// is a plain description of the choice, so [`Self::decide`] can turn it
    /// into an action without any access to the hook.
    fn consider(&self, error: &CodecError, origin: DecodeOrigin, consumed: usize) -> Selection {
        let context = self.error_context();
        let selected = self.select_policy(error, &context);
        let applied = self.apply_fallbacks(selected, origin, consumed);
        let quarantine = match applied {
            RecoveryPolicy::Quarantine => self.quarantine_for(error, &context),
            _ => Duration::ZERO,
        };

        Selection {
            selected,
            applied,
            quarantine,
            extends_streak: is_budgeted_drop(selected, origin, consumed),
        }
    }

    /// Decide what to do with one failure the hook has already answered.
    ///
    /// A pure query: it reads the drop streak and writes nothing. It cannot
    /// consult the hook, store a streak, report, emit a metric, or log, because
    /// [`Self::consider`] has already resolved every input it needs.
    const fn decide(&self, selection: Selection) -> Decision {
        let Selection {
            selected,
            applied,
            quarantine,
            extends_streak,
        } = selection;
        let consecutive_drops = if extends_streak {
            self.consecutive_drops.saturating_add(1)
        } else {
            self.consecutive_drops
        };

        Decision {
            policies: ReportedPolicies { selected, applied },
            action: action_for(applied, quarantine),
            consecutive_drops,
        }
    }

    /// Ask the hook how long a quarantine should pause.
    ///
    /// The configured duration is the fallback for a hook that does not answer.
    /// An applied quarantine is only reachable with a hook installed, because
    /// [`Self::select_policy`] forces a disconnect otherwise.
    fn quarantine_for(&self, error: &CodecError, context: &CodecErrorContext) -> Duration {
        self.hook.map_or(self.config.quarantine_duration, |hook| {
            hook.quarantine_duration(error, context)
        })
    }

    /// Store the drop streak a decision computed.
    const fn record(&mut self, decision: Decision) {
        self.consecutive_drops = decision.consecutive_drops;
    }

    /// Carry out the two commands a decision requires.
    ///
    /// Recording comes first so the next failure sees the updated streak, then
    /// the decision is reported through the single path that touches the hook,
    /// the metric, and the log.
    fn record_and_report(&mut self, error: &CodecError, decision: Decision) {
        self.record(decision);
        self.publish(error, decision.policies);
    }

    /// Report one decided failure to the hook, the metric, and the log.
    ///
    /// The only place that calls `on_error` and `inc_codec_error`, so a decision
    /// taken by [`Self::decide`] cannot emit anything on its own.
    fn publish(&self, error: &CodecError, policies: ReportedPolicies) {
        let ReportedPolicies { selected, applied } = policies;
        if let Some(hook) = self.hook {
            hook.on_error(error, &self.error_context(), applied);
        }
        crate::metrics::inc_codec_error(error.error_type(), applied.as_str());
        self.log_failure(error, selected, applied);
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
    /// no bytes would repeat forever; both force a disconnect for a policy that
    /// discards the frame. A drop streak beyond the configured maximum escalates
    /// in the same way.
    fn apply_fallbacks(
        &self,
        selected: RecoveryPolicy,
        origin: DecodeOrigin,
        consumed: usize,
    ) -> RecoveryPolicy {
        if !is_budgeted_drop(selected, origin, consumed) {
            return match selected {
                RecoveryPolicy::Drop | RecoveryPolicy::Quarantine
                    if origin == DecodeOrigin::DecodeEof || consumed == 0 =>
                {
                    RecoveryPolicy::Disconnect
                }
                other => other,
            };
        }
        if self.consecutive_drops.saturating_add(1) > self.config.max_consecutive_drops {
            RecoveryPolicy::Disconnect
        } else {
            RecoveryPolicy::Drop
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
#[path = "recovery_dispatch_tests.rs"]
mod dispatch_tests;

// The read step's tests drive the same hook and metric that recovery reports
// through, so these fixtures are shared across the inbound module rather than
// rebuilt in each test tree.
#[cfg(test)]
#[path = "recovery_test_support.rs"]
pub(in crate::app::inbound_handler) mod test_support;

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
