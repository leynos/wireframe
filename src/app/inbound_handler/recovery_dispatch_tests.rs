//! Tests for settling a dispatch against a prefetched decoder failure.
//!
//! Two exits abandon the failure buffered behind a response: the response ends
//! the connection, or shutdown cancels the next read before the buffer is
//! drained. These tests drive [`settle_dispatch`], the read step, and
//! [`report_pending_failure`] exactly as the connection loop does, and assert
//! what the hook and the metric observe for each outcome.

use std::{io, time::Duration};

#[cfg(feature = "metrics")]
use metrics_util::debugging::DebuggingRecorder;
use rstest::rstest;
use tokio_util::sync::CancellationToken;

use super::{test_support::*, *};
use crate::app::{
    frame_handling::Dispatch,
    inbound_handler::frames::{InboundFrames, NextFrame},
    recovery_codec::DecodedItem,
};

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

/// Drive a continuing response, then end the read by cancelling shutdown.
///
/// This is the read loop's other exit: [`settle_dispatch`] keeps the buffered
/// failure for the read step, and shutdown then wins the read race before the
/// buffer is drained. The item must survive that read, because the loop reports
/// it from the branch that returns `None`.
async fn continue_then_end_read(
    inbound: &mut InboundFrames<DecodedItem<u8>, u8>,
    coordinator: &RecoveryCoordinator<'_>,
) {
    let settled = settle_dispatch(Ok(Dispatch::Continue), inbound, coordinator)
        .expect("a continuing dispatch settles without an error");
    assert_eq!(settled, Dispatch::Continue);
    assert!(
        inbound.pending.is_some(),
        "the read step still owns this failure's policy"
    );

    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let mut reader = futures::stream::empty();
    let read = inbound
        .next_frame(&mut reader, 50, &shutdown)
        .await
        .expect("a cancelled read ends without an error");

    assert!(
        matches!(read, NextFrame::Closed),
        "shutdown ends the read"
    );
    assert!(
        inbound.pending.is_some(),
        "the buffered failure survives the cancelled read"
    );
}

#[tokio::test]
async fn shutdown_ended_read_reports_the_buffered_failure_as_a_disconnect() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let mut inbound = inbound_with_pending_failure();

    continue_then_end_read(&mut inbound, &coordinator).await;
    report_pending_failure(&mut inbound, &coordinator);

    assert!(
        inbound.pending.is_none(),
        "the reported failure must be taken exactly once"
    );
    assert_eq!(hook.error_types(), vec!["protocol"]);
    assert_eq!(
        hook.applied_policies(),
        vec![RecoveryPolicy::Disconnect],
        "a closing connection applies no hook-selected policy"
    );
}

#[cfg(feature = "metrics")]
#[tokio::test]
async fn shutdown_ended_read_records_a_disconnect_metric() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let mut inbound = inbound_with_pending_failure();

    // Only the report step records; the read loop must leave the failure for it.
    continue_then_end_read(&mut inbound, &coordinator).await;
    metrics::with_local_recorder(&recorder, || {
        report_pending_failure(&mut inbound, &coordinator);
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(policy_count(&samples, "disconnect"), 1);
    assert_eq!(
        policy_count(&samples, "drop"),
        0,
        "a policy that was never applied must not be recorded"
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
