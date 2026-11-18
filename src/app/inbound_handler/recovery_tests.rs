//! Tests for decoder-failure recovery evaluation and reporting.

use std::{io, net::SocketAddr, sync::Mutex, time::Duration};

#[cfg(feature = "metrics")]
use metrics::{SharedString, Unit};
#[cfg(feature = "metrics")]
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use rstest::rstest;

use super::*;
use crate::{
    app::recovery_codec::{DecodeFailure, DecodeOrigin},
    codec::ProtocolError,
};

/// Bytes consumed by the default test failure.
const CONSUMED: usize = 8;

/// One observed `on_error` call.
struct HookCall {
    /// Policy the connection loop applied.
    policy: RecoveryPolicy,
    /// Error category the connection loop classified.
    error_type: &'static str,
    /// Peer address carried in the context, when one was known.
    peer_address: Option<SocketAddr>,
}

/// Hook that records each `on_error` call and answers with fixed settings.
struct RecordingHook {
    /// Policy returned from `recovery_policy`.
    selected: RecoveryPolicy,
    /// Duration returned from `quarantine_duration`.
    quarantine: Duration,
    /// Callbacks observed by `on_error`.
    calls: Mutex<Vec<HookCall>>,
}

impl RecordingHook {
    /// Build a hook that selects `selected` and pauses for `quarantine`.
    fn new(selected: RecoveryPolicy, quarantine: Duration) -> Self {
        Self {
            selected,
            quarantine,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Return the policies `on_error` was called with.
    fn applied_policies(&self) -> Vec<RecoveryPolicy> {
        self.calls
            .lock()
            .expect("hook call log lock")
            .iter()
            .map(|call| call.policy)
            .collect()
    }

    /// Return the error types `on_error` was called with.
    fn error_types(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .expect("hook call log lock")
            .iter()
            .map(|call| call.error_type)
            .collect()
    }

    /// Return the peer addresses `on_error` was called with.
    fn peer_addresses(&self) -> Vec<Option<SocketAddr>> {
        self.calls
            .lock()
            .expect("hook call log lock")
            .iter()
            .map(|call| call.peer_address)
            .collect()
    }
}

impl RecoveryPolicyHook for RecordingHook {
    fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        self.selected
    }

    fn quarantine_duration(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> Duration {
        self.quarantine
    }

    fn on_error(&self, error: &CodecError, ctx: &CodecErrorContext, policy: RecoveryPolicy) {
        self.calls
            .lock()
            .expect("hook call log lock")
            .push(HookCall {
                policy,
                error_type: error.error_type(),
                peer_address: ctx.peer_address,
            });
    }
}

/// Classified protocol failure as a decoder would report it.
fn protocol_failure() -> DecodeFailure {
    let error = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 7 });
    DecodeFailure::new(error.into(), DecodeOrigin::Decode, CONSUMED)
}

/// Configuration with a small drop budget so escalation is easy to reach.
fn tight_config() -> RecoveryConfig {
    RecoveryConfig::default()
        .max_consecutive_drops(2)
        .quarantine_duration(Duration::from_millis(5))
}

#[cfg(feature = "metrics")]
type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// Total count recorded for the codec-error counter under one policy label.
#[cfg(feature = "metrics")]
fn policy_count(samples: &[MetricSample], expected: &str) -> u64 {
    samples
        .iter()
        .filter(|(key, ..)| key.key().name() == crate::metrics::CODEC_ERRORS)
        .filter(|(key, ..)| {
            key.key()
                .labels()
                .any(|label| label.key() == "recovery_policy" && label.value() == expected)
        })
        .map(|(_, _, _, value)| match value {
            DebugValue::Counter(count) => *count,
            _ => 0,
        })
        .sum()
}

#[test]
fn no_hook_disconnects_without_reporting() {
    let mut coordinator = RecoveryCoordinator::new(None, tight_config());
    let outcome = coordinator.evaluate(protocol_failure());
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

    let outcome = coordinator.evaluate(protocol_failure());

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

    let outcome = coordinator.evaluate(protocol_failure());

    assert_eq!(outcome.action, RecoveryAction::Drop);
    assert_eq!(hook.peer_addresses(), vec![Some(address)]);
}

#[rstest]
fn hook_receives_no_peer_address_without_a_transport_address() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    let _ = coordinator.evaluate(protocol_failure());

    assert_eq!(hook.peer_addresses(), vec![None]);
}

#[rstest]
fn eof_failures_always_disconnect() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let error = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 7 });
    let failure = DecodeFailure::new(error.into(), DecodeOrigin::DecodeEof, CONSUMED);

    let outcome = coordinator.evaluate(failure);

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

    let outcome = coordinator.evaluate(failure);

    assert_eq!(outcome.action, RecoveryAction::Disconnect);
    assert_eq!(hook.error_types(), vec!["io"]);
}

#[rstest]
fn drop_budget_allows_the_limit_then_escalates() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    let actions: Vec<RecoveryAction> = (0..3)
        .map(|_| coordinator.evaluate(protocol_failure()).action)
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

    let _ = coordinator.evaluate(protocol_failure());
    let _ = coordinator.evaluate(protocol_failure());
    coordinator.record_decoded_frame();
    let outcome = coordinator.evaluate(protocol_failure());

    assert_eq!(outcome.action, RecoveryAction::Drop);
}

#[rstest]
fn hook_controls_the_quarantine_duration() {
    let pause = Duration::from_millis(37);
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, pause);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());

    let outcome = coordinator.evaluate(protocol_failure());

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

    let outcome = coordinator.evaluate(failure);

    assert_eq!(outcome.action, RecoveryAction::Disconnect);
    assert_eq!(outcome.error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(outcome.error.to_string(), "short read");
    assert_eq!(
        hook.error_types(),
        vec!["io"],
        "an untyped failure is presented to the hook as an I/O error"
    );
}

#[cfg(feature = "metrics")]
#[test]
fn each_failure_records_one_codec_error_with_the_applied_policy() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    metrics::with_local_recorder(&recorder, || {
        let mut coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
        let _ = coordinator.evaluate(protocol_failure());
        let _ = coordinator.evaluate(protocol_failure());
        let _ = coordinator.evaluate(protocol_failure());
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
        let _ = coordinator.evaluate(protocol_failure());
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

#[rstest]
fn pending_failure_is_reported_before_the_loop_exits() {
    let hook = RecordingHook::new(RecoveryPolicy::Disconnect, Duration::from_millis(5));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let mut inbound: InboundFrames<DecodedItem<u8>, u8> = InboundFrames {
        pending: Some(DecodedItem::Failure(protocol_failure())),
        pipeline: FramePipeline::new(None),
        message_assembly: None,
    };

    report_pending_failure(&mut inbound, &mut coordinator);

    assert!(inbound.pending.is_none(), "the reported item must be taken");
    assert_eq!(
        hook.error_types(),
        vec!["protocol"],
        "the buffered failure must still reach on_error"
    );
}
