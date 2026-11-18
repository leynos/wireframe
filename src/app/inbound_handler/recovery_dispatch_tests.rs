//! Tests for settling a dispatch against a prefetched decoder failure.
//!
//! A response that ends the connection abandons the failure buffered behind it,
//! so these tests drive [`settle_dispatch`] exactly as the read loop does and
//! assert what the hook and the metric observe for each outcome.

use std::{io, time::Duration};

#[cfg(feature = "metrics")]
use metrics_util::debugging::DebuggingRecorder;
use rstest::rstest;

use super::{test_support::*, *};
use crate::app::frame_handling::Dispatch;

/// One terminal dispatch outcome whose buffered failure must still be reported.
#[derive(Clone, Copy, Debug)]
enum TerminalOutcome {
    /// The protocol cancelled the response stream.
    Stop,
    /// The dispatch step itself failed.
    Error,
}

impl TerminalOutcome {
    /// The dispatch result the loop settles for this outcome.
    fn settled(self) -> io::Result<Dispatch> {
        match self {
            Self::Stop => Ok(Dispatch::Stop),
            Self::Error => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "response write failed",
            )),
        }
    }

    /// Whether settling this outcome surfaces an error to the loop.
    const fn is_error(self) -> bool { matches!(self, Self::Error) }
}

#[rstest]
#[case::stop(TerminalOutcome::Stop)]
#[case::error(TerminalOutcome::Error)]
fn terminal_dispatch_reports_the_pending_failure_as_a_disconnect(#[case] outcome: TerminalOutcome) {
    // The hook would have dropped this failure, but the connection is already
    // ending, so reporting a drop would describe an action that never ran.
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let mut inbound = inbound_with_pending_failure();

    let settled = settle_dispatch(outcome.settled(), &mut inbound, &coordinator);

    assert_eq!(
        settled.is_err(),
        outcome.is_error(),
        "settling must not swallow or invent dispatch errors"
    );
    assert!(inbound.pending.is_none(), "the reported item must be taken");
    assert_eq!(hook.error_types(), vec!["protocol"]);
    assert_eq!(
        hook.applied_policies(),
        vec![RecoveryPolicy::Disconnect],
        "on_error must see the disconnect that actually happened"
    );
}

#[rstest]
fn continuing_dispatch_leaves_the_pending_failure_for_the_read_step() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let mut inbound = inbound_with_pending_failure();

    let settled = settle_dispatch(Ok(Dispatch::Continue), &mut inbound, &coordinator)
        .expect("a continuing dispatch settles without an error");

    assert_eq!(settled, Dispatch::Continue);
    assert!(
        inbound.pending.is_some(),
        "the read step still owns this failure's policy"
    );
    assert!(
        hook.applied_policies().is_empty(),
        "nothing may be reported before the policy is applied"
    );
}

#[cfg(feature = "metrics")]
#[rstest]
#[case::stop(TerminalOutcome::Stop)]
#[case::error(TerminalOutcome::Error)]
fn terminal_dispatch_records_a_disconnect_metric(#[case] outcome: TerminalOutcome) {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let mut inbound = inbound_with_pending_failure();
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    metrics::with_local_recorder(&recorder, || {
        // The outcome itself is asserted by the callback test; here only the
        // recording matters.
        drop(settle_dispatch(
            outcome.settled(),
            &mut inbound,
            &coordinator,
        ));
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(policy_count(&samples, "disconnect"), 1);
    assert_eq!(
        policy_count(&samples, "drop"),
        0,
        "a policy that was never applied must not be recorded"
    );
}
