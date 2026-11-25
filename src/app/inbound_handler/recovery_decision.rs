//! The values a decoder-failure decision is made of.
//!
//! [`RecoveryCoordinator`](super::RecoveryCoordinator) asks the hook once and
//! describes the answer as a [`Selection`]; its pure `decide` step turns that
//! into a [`Decision`]. These types carry data only, so the boundary between
//! consulting the hook and acting on its answer stays visible in the types
//! rather than in a comment.

use std::time::Duration;

use crate::{app::recovery_codec::DecodeOrigin, codec::RecoveryPolicy};

/// What the connection loop should do after a decoder failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app::inbound_handler) enum RecoveryAction {
    /// Discard the malformed frame and keep reading.
    Drop,
    /// Pause the connection for this duration, then keep reading.
    Quarantine(Duration),
    /// Stop reading and return the original error to the teardown path.
    Disconnect,
}

/// The hook's selection and the policy actually applied to one failure.
///
/// They differ whenever a fallback overrides the hook, and a failure the
/// connection can no longer act on reports neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app::inbound_handler) struct ReportedPolicies {
    /// Policy selected before any fallback; logged for comparison.
    pub(in crate::app::inbound_handler) selected: RecoveryPolicy,
    /// Policy applied by the loop, reported to the hook and the metric.
    pub(in crate::app::inbound_handler) applied: RecoveryPolicy,
}

impl ReportedPolicies {
    /// The single policy a closing connection reports.
    ///
    /// No hook policy can take effect, so the selection mirrors the applied
    /// disconnect rather than naming a policy that was never considered.
    pub(in crate::app::inbound_handler) const fn terminal() -> Self {
        Self {
            selected: RecoveryPolicy::Disconnect,
            applied: RecoveryPolicy::Disconnect,
        }
    }
}

/// What the hook chose for one failure and what the connection will apply.
///
/// Gathering this is the only step that consults the hook. It is then a plain
/// input to the pure decision, which has no way to reach the hook, the metric,
/// or the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app::inbound_handler) struct Selection {
    /// Policy selected before any fallback; logged for comparison.
    pub(in crate::app::inbound_handler) selected: RecoveryPolicy,
    /// Policy the connection loop applies, reported to the hook and the metric.
    pub(in crate::app::inbound_handler) applied: RecoveryPolicy,
    /// Pause the hook asked for, meaningful only for a quarantine.
    pub(in crate::app::inbound_handler) quarantine: Duration,
    /// Whether this failure extends the connection's drop streak.
    pub(in crate::app::inbound_handler) extends_streak: bool,
}

/// One decided failure, before the connection has recorded or reported it.
///
/// The pure decision step returns this without storing the drop streak, calling
/// the hook's `on_error`, emitting a metric, or logging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app::inbound_handler) struct Decision {
    /// The hook's selection and the policy the connection applied.
    pub(in crate::app::inbound_handler) policies: ReportedPolicies,
    /// Action the read loop must perform.
    pub(in crate::app::inbound_handler) action: RecoveryAction,
    /// Drop streak the connection must store for the next failure.
    pub(in crate::app::inbound_handler) consecutive_drops: u32,
}

/// Translate an applied policy into the action the loop performs.
///
/// The pause is meaningful only for a quarantine, so every other policy ignores
/// it.
pub(in crate::app::inbound_handler) const fn action_for(
    applied: RecoveryPolicy,
    quarantine: Duration,
) -> RecoveryAction {
    match applied {
        RecoveryPolicy::Drop => RecoveryAction::Drop,
        RecoveryPolicy::Disconnect => RecoveryAction::Disconnect,
        RecoveryPolicy::Quarantine => RecoveryAction::Quarantine(quarantine),
    }
}

/// Whether this failure extends the connection's drop streak.
///
/// Only a drop the connection can actually apply counts: an EOF, or a failure
/// that consumed no bytes, disconnects whatever the hook selected, and a
/// selection other than drop never contributes to the drop budget.
pub(in crate::app::inbound_handler) const fn is_budgeted_drop(
    selected: RecoveryPolicy,
    origin: DecodeOrigin,
    consumed: usize,
) -> bool {
    matches!(selected, RecoveryPolicy::Drop)
        && !matches!(origin, DecodeOrigin::DecodeEof)
        && consumed > 0
}
