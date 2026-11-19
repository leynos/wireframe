//! Tests for decoder-failure recovery evaluation.
//!
//! These cover how one failure is classified, escalated, and reported, that
//! deciding alone changes nothing, and what a quarantine pause purges. The
//! loop-level tests that settle a dispatch live beside
//! [`super::dispatch_tests`].

use std::{io, net::SocketAddr, num::NonZeroUsize, time::Duration};

#[cfg(feature = "metrics")]
use metrics_util::debugging::DebuggingRecorder;
use rstest::rstest;

use super::{test_support::*, *};
use crate::{
    app::Envelope,
    codec::{ProtocolError, RecoveryPolicy},
    fragment::{
        FragmentHeader,
        FragmentIndex,
        FragmentationConfig,
        MessageId,
        encode_fragment_payload,
    },
};

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

/// A decision is a query: it must leave the hook, the counter, and the metric
/// untouched, and so must reach the same answer however often it is asked.
#[cfg(feature = "metrics")]
#[test]
fn deciding_changes_no_metric_hook_or_counter_state() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let error = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 7 });

    metrics::with_local_recorder(&recorder, || {
        for _ in 0..3 {
            let decision = coordinator.decide(&error, DecodeOrigin::Decode, CONSUMED);
            assert_eq!(decision.action, RecoveryAction::Drop);
        }
    });

    assert!(
        hook.applied_policies().is_empty(),
        "deciding must not report through on_error"
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

#[tokio::test]
async fn shutdown_ends_a_quarantine_pause_early() {
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, Duration::from_secs(3600));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let mut pipeline: FramePipeline<()> = FramePipeline::new(None);
    let mut message_assembly = None;

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        coordinator.apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                shutdown: &shutdown,
            },
        ),
    )
    .await;

    assert!(
        matches!(result, Ok(None)),
        "a cancelled shutdown must end the pause without a disconnect"
    );
}

#[tokio::test]
async fn quarantine_pause_completes_without_shutdown() {
    let pause = Duration::from_millis(5);
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, pause);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    let mut pipeline: FramePipeline<()> = FramePipeline::new(None);
    let mut message_assembly = None;

    let result = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                shutdown: &shutdown,
            },
        )
        .await;

    assert!(result.is_none(), "quarantine keeps the connection alive");
}

/// A pause must discard partial fragments that expired while it ran.
///
/// A buffer left over from before the pause would otherwise be reassembled
/// with fragments read afterwards, so the connection would report a frame
/// assembled from two different eras of the stream.
#[tokio::test]
async fn quarantine_pause_purges_expired_partial_fragments() {
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, Duration::ZERO);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    let mut pipeline: FramePipeline<Envelope> = FramePipeline::new(Some(fragment_config()));
    buffer_partial_fragment(&mut pipeline);
    assert!(
        pipeline.fragmentation_buffered_bytes() > 0,
        "the fixture must leave a partial fragment buffered"
    );
    let mut message_assembly = None;

    let result = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                shutdown: &shutdown,
            },
        )
        .await;

    assert!(result.is_none(), "a quarantine pause keeps reading");
    assert_eq!(
        pipeline.fragmentation_buffered_bytes(),
        0,
        "the pause must purge the expired partial fragment"
    );
}

/// Fragmentation whose partial buffers expire immediately, so a purge is
/// observable without waiting for a clock.
fn fragment_config() -> FragmentationConfig {
    FragmentationConfig::for_frame_budget(
        1024,
        NonZeroUsize::new(4096).expect("a non-zero message limit"),
        Duration::ZERO,
    )
    .expect("the frame budget supports fragmentation")
}

/// Feed the pipeline the first fragment of a two-fragment message.
fn buffer_partial_fragment(pipeline: &mut FramePipeline<Envelope>) {
    let header = FragmentHeader::new(MessageId::new(9), FragmentIndex::zero(), false);
    let payload = encode_fragment_payload(header, &[1_u8, 2]).expect("the fragment encodes");
    let partial = Envelope::new(9, None, payload);
    let state = pipeline
        .fragmentation_mut()
        .expect("the fixture enables fragmentation");
    let _ = state
        .reassemble(partial)
        .expect("the first fragment is buffered");
}

#[path = "recovery_properties.rs"]
mod properties;
