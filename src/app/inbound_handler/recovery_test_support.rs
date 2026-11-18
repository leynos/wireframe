//! Shared fixtures for the decoder-failure recovery tests.
//!
//! The evaluation tests and the dispatch-settling tests observe the same hook
//! callbacks and metric labels, so the recording hook, the bounded recovery
//! configuration, and the state builders live here rather than being
//! duplicated in each test module.

use std::{net::SocketAddr, sync::Mutex, time::Duration};

#[cfg(feature = "metrics")]
use metrics::{SharedString, Unit};
#[cfg(feature = "metrics")]
use metrics_util::{CompositeKey, debugging::DebugValue};

use super::*;
use crate::{
    app::{
        inbound_handler::frames::InboundFrames,
        recovery_codec::{DecodeFailure, DecodeOrigin, DecodedItem},
    },
    codec::ProtocolError,
};

/// Bytes consumed by the default test failure.
pub(super) const CONSUMED: usize = 8;

/// One observed `on_error` call.
pub(super) struct HookCall {
    /// Policy the connection loop applied.
    policy: RecoveryPolicy,
    /// Error category the connection loop classified.
    error_type: &'static str,
    /// Peer address carried in the context, when one was known.
    peer_address: Option<SocketAddr>,
}

/// Hook that records each `on_error` call and answers with fixed settings.
pub(super) struct RecordingHook {
    /// Policy returned from `recovery_policy`.
    selected: RecoveryPolicy,
    /// Duration returned from `quarantine_duration`.
    quarantine: Duration,
    /// Callbacks observed by `on_error`.
    calls: Mutex<Vec<HookCall>>,
}

impl RecordingHook {
    /// Build a hook that selects `selected` and pauses for `quarantine`.
    pub(super) fn new(selected: RecoveryPolicy, quarantine: Duration) -> Self {
        Self {
            selected,
            quarantine,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Return the policies `on_error` was called with.
    pub(super) fn applied_policies(&self) -> Vec<RecoveryPolicy> {
        self.calls
            .lock()
            .expect("hook call log lock")
            .iter()
            .map(|call| call.policy)
            .collect()
    }

    /// Return the error types `on_error` was called with.
    pub(super) fn error_types(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .expect("hook call log lock")
            .iter()
            .map(|call| call.error_type)
            .collect()
    }

    /// Return the peer addresses `on_error` was called with.
    pub(super) fn peer_addresses(&self) -> Vec<Option<SocketAddr>> {
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
pub(super) fn protocol_failure() -> DecodeFailure {
    let error = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 7 });
    DecodeFailure::new(error.into(), DecodeOrigin::Decode, CONSUMED)
}

/// Configuration with a small drop budget so escalation is easy to reach.
pub(super) fn tight_config() -> RecoveryConfig {
    RecoveryConfig::default()
        .max_consecutive_drops(2)
        .quarantine_duration(Duration::from_millis(5))
}

/// Inbound state holding one failure prefetched before a response finished.
pub(super) fn inbound_with_pending_failure() -> InboundFrames<DecodedItem<u8>, u8> {
    InboundFrames {
        pending: Some(DecodedItem::Failure(protocol_failure())),
        pipeline: FramePipeline::new(None),
        message_assembly: None,
    }
}

/// One recorded metric sample.
#[cfg(feature = "metrics")]
pub(super) type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// Total count recorded for the codec-error counter under one policy label.
#[cfg(feature = "metrics")]
pub(super) fn policy_count(samples: &[MetricSample], expected: &str) -> u64 {
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
