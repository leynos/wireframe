//! Tests for decoder-failure recovery evaluation.
//!
//! These cover how one failure is classified, escalated, and reported, that
//! deciding alone changes nothing, and what a quarantine pause purges. The
//! loop-level tests that settle a dispatch live beside
//! [`super::dispatch_tests`].

use std::{io, net::SocketAddr, time::Duration};

#[cfg(feature = "metrics")]
use metrics_util::debugging::DebuggingRecorder;
use rstest::rstest;

use super::{test_support::*, *};
use crate::codec::{ProtocolError, RecoveryPolicy};

#[test]
fn no_hook_disconnects_without_reporting() {
    let mut coordinator = RecoveryCoordinator::new(None, tight_config());
    let outcome = coordinator.record_failure(protocol_failure());
    assert_eq!(outcome.action, RecoveryAction::Disconnect);
    assert_eq!(
        outcome.error.kind(),
        io::ErrorKind::InvalidData,
        "the original error must reach the caller unchanged"
    );
}

#[rstest]
fn hook_selected_drop_is_applied() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    let outcome = coordinator.record_failure(protocol_failure());

    assert_eq!(outcome.action, RecoveryAction::Drop);
    assert_eq!(hook.applied_policies(), vec![RecoveryPolicy::Drop]);
    assert_eq!(hook.error_types(), vec!["protocol"]);
}

#[rstest]
fn hook_receives_the_connection_peer_address() {
    let address = SocketAddr::from(([127, 0, 0, 1], 40_000));
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator =
        RecoveryCoordinator::new(Some(&hook), tight_config()).with_peer_address(Some(address));

    let outcome = coordinator.record_failure(protocol_failure());

    assert_eq!(outcome.action, RecoveryAction::Drop);
    assert_eq!(hook.peer_addresses(), vec![Some(address)]);
}

#[rstest]
fn hook_receives_no_peer_address_without_a_transport_address() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    let _ = coordinator.record_failure(protocol_failure());

    assert_eq!(hook.peer_addresses(), vec![None]);
}

#[rstest]
fn eof_failures_always_disconnect() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let error = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 7 });
    let failure = DecodeFailure::new(error.into(), DecodeOrigin::DecodeEof, CONSUMED);

    let outcome = coordinator.record_failure(failure);

    assert_eq!(outcome.action, RecoveryAction::Disconnect);
    assert_eq!(
        hook.applied_policies(),
        vec![RecoveryPolicy::Disconnect],
        "on_error must see the applied policy, not the selection"
    );
}

#[rstest]
fn unclassified_failure_without_consumption_disconnects(
    #[values(RecoveryPolicy::Drop, RecoveryPolicy::Quarantine)] selected: RecoveryPolicy,
) {
    let hook = RecordingHook::new(selected, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let failure = DecodeFailure::new(
        io::Error::new(io::ErrorKind::InvalidData, "unconsumed frame"),
        DecodeOrigin::Decode,
        0,
    );

    let outcome = coordinator.record_failure(failure);

    assert_eq!(outcome.action, RecoveryAction::Disconnect);
    assert_eq!(hook.error_types(), vec!["io"]);
}

#[rstest]
fn drop_budget_allows_the_limit_then_escalates() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    let actions: Vec<RecoveryAction> = (0..3)
        .map(|_| coordinator.record_failure(protocol_failure()).action)
        .collect();

    assert_eq!(
        actions,
        vec![
            RecoveryAction::Drop,
            RecoveryAction::Drop,
            RecoveryAction::Disconnect
        ],
        "the configured maximum is inclusive, one more disconnects"
    );
}

#[rstest]
fn decoded_frame_restarts_the_drop_budget() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    let _ = coordinator.record_failure(protocol_failure());
    let _ = coordinator.record_failure(protocol_failure());
    coordinator.record_decoded_frame();
    let outcome = coordinator.record_failure(protocol_failure());

    assert_eq!(outcome.action, RecoveryAction::Drop);
}

#[rstest]
fn hook_controls_the_quarantine_duration() {
    let pause = Duration::from_millis(37);
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, pause);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());

    let outcome = coordinator.record_failure(protocol_failure());

    assert_eq!(outcome.action, RecoveryAction::Quarantine(pause));
}

#[rstest]
fn untyped_error_keeps_its_kind_and_payload() {
    let hook = RecordingHook::new(RecoveryPolicy::Disconnect, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let failure = DecodeFailure::new(
        io::Error::new(io::ErrorKind::UnexpectedEof, "short read"),
        DecodeOrigin::Decode,
        CONSUMED,
    );

    let outcome = coordinator.record_failure(failure);

    assert_eq!(outcome.action, RecoveryAction::Disconnect);
    assert_eq!(outcome.error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(outcome.error.to_string(), "short read");
    assert_eq!(
        hook.error_types(),
        vec!["io"],
        "an untyped failure is presented to the hook as an I/O error"
    );
}

/// A decision is a query: it must leave the hook and its own state untouched,
/// emit nothing, and answer the same way however often it is asked.
///
/// The hook is stateful, so a single consultation would show up in its counter;
/// the metric recorder is the injected observer, so any report would appear in
/// the snapshot.
#[cfg(feature = "metrics")]
#[test]
fn deciding_changes_no_metric_hook_or_counter_state() {
    let hook = ScriptedHook::new(vec![RecoveryPolicy::Drop; 3]);
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let selection = Selection {
        selected: RecoveryPolicy::Drop,
        applied: RecoveryPolicy::Drop,
        quarantine: Duration::ZERO,
        extends_streak: true,
    };

    metrics::with_local_recorder(&recorder, || {
        for _ in 0..3 {
            let decision = coordinator.decide(selection);
            assert_eq!(decision.action, RecoveryAction::Drop);
            assert_eq!(
                decision.consecutive_drops, 1,
                "the decision reads the streak without storing it"
            );
        }
    });

    assert_eq!(
        hook.consultations(),
        0,
        "deciding must not consult the hook"
    );
    assert_eq!(
        coordinator.consecutive_drops, 0,
        "deciding must not store the drop streak"
    );
    assert!(
        snapshotter.snapshot().into_vec().is_empty(),
        "deciding must not emit a metric"
    );
}

/// Metrics were asserted by their own test; here only the label counts matter.
#[cfg(feature = "metrics")]
#[test]
fn each_failure_records_one_codec_error_with_the_applied_policy() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    metrics::with_local_recorder(&recorder, || {
        let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
        let _ = coordinator.record_failure(protocol_failure());
        let _ = coordinator.record_failure(protocol_failure());
        let _ = coordinator.record_failure(protocol_failure());
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(policy_count(&samples, "drop"), 2);
    assert_eq!(policy_count(&samples, "disconnect"), 1);
}

#[cfg(feature = "metrics")]
#[test]
fn no_hook_records_a_disconnect_for_each_failure() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    metrics::with_local_recorder(&recorder, || {
        let mut coordinator = RecoveryCoordinator::new(None, tight_config());
        let _ = coordinator.record_failure(protocol_failure());
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(policy_count(&samples, "disconnect"), 1);
    assert_eq!(policy_count(&samples, "drop"), 0);
}

#[path = "recovery_quarantine_tests.rs"]
mod quarantine;

#[path = "recovery_properties.rs"]
mod properties;
