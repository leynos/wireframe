//! Counted and warned idle-grace abandonment of a push write.
//!
//! The graceful drain bounds each write separately, so a peer that stopped
//! reading must be reported as the idle grace expiring rather than as a server
//! stop. These cases hold the shutdown token open so only the grace can end the
//! write, and then read what the operator would see.

use std::net::SocketAddr;

use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use wireframe_testing::{LoggerHandle, logger};

use super::*;
use crate::{
    app::PushWriteDiagnostics,
    metrics::PUSH_WRITES_ABANDONED,
    push::PushPriority,
    session::ConnectionId,
};

/// One recorded metric sample, as the debugging recorder reports it.
type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// Tally the abandonment counter and read back the reasons it was labelled with.
fn abandonment(samples: &[MetricSample]) -> (u64, Vec<String>) {
    let mut count = 0;
    let mut reasons = Vec::new();
    for (key, _, _, value) in samples
        .iter()
        .filter(|(key, ..)| key.key().name() == PUSH_WRITES_ABANDONED)
    {
        if let DebugValue::Counter(recorded) = value {
            count += *recorded;
        }
        reasons.extend(
            key.key()
                .labels()
                .filter(|label| label.key() == "reason")
                .map(|label| label.value().to_owned()),
        );
    }
    (count, reasons)
}

/// Collect the captured log records that name one connection.
///
/// The log buffer is process-wide, so only records naming this connection count
/// as this test's abandonment.
fn warnings_for(log: &mut LoggerHandle, connection_id: ConnectionId) -> Vec<String> {
    std::iter::from_fn(|| log.pop())
        .map(|record| record.args().to_owned())
        .filter(|message| message.contains(&connection_id.to_string()))
        .collect()
}

/// Assert the abandonment warning carries every operator-visible field.
fn assert_warning_names(warning: &str, peer: SocketAddr) {
    let peer_field = format!("peer_addr=Some({peer})");
    let elapsed_field = format!("elapsed_ms={}", PEER_CLOSED_RESPONSE_GRACE.as_millis());
    for fragment in [
        peer_field.as_str(),
        r#"priority="high""#,
        r#"reason="idle_grace_expired""#,
        elapsed_field.as_str(),
    ] {
        assert!(
            warning.contains(fragment),
            "the abandonment warning must name {fragment}; captured: {warning}"
        );
    }
}

/// An idle grace that abandons a drain write is counted and warned about.
///
/// The peer closed its write half before the drain, so `finish_write` bounds
/// the write by the grace from its first poll. The shutdown token is never
/// cancelled, so only the grace can end it: the abandonment must be classified
/// `idle_grace_expired` rather than `shutdown`, counted once, and reported with
/// the warning naming the connection. Removing the recording call leaves both
/// the counter and the warning unrecorded.
#[cfg(feature = "metrics")]
#[rstest::rstest]
#[tokio::test(start_paused = true)]
async fn an_idle_grace_abandoned_drain_write_is_counted_and_warned(
    #[from(blocked_transport)] mut harness: TransportRig,
) {
    let connection_id = ConnectionId::new(4_301);
    let peer = SocketAddr::from(([127, 0, 0, 1], 9_044));
    let shutdown = CancellationToken::new();
    let peer_eof = CancellationToken::new();
    let mut pending_frame = None;
    let mut reader = stream::pending::<io::Result<Bytes>>();
    let mut batch = PushBatch {
        frames: Vec::new(),
        priority: Some(PushPriority::High),
        reader: &mut reader,
        pending_frame: &mut pending_frame,
        peer_eof: &peer_eof,
        shutdown: &shutdown,
    };
    let mut log = logger();
    log.clear();

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let outcome = {
        let _recorder = ::metrics::set_default_local_recorder(&recorder);
        // `peer_closed` skips the observation select, so the blocked write is
        // handed straight to `finish_write` under the uncancelled token.
        let mut transport =
            harness.transport_with(PushWriteDiagnostics::new(Some(connection_id), Some(peer)));
        write_frame(
            &mut transport,
            Bytes::from_static(b"stalled"),
            &mut batch,
            true,
        )
        .await
    }
    .expect("an idle-grace abandonment is not an error");
    assert!(!outcome.may_continue(), "the connection must stop");
    assert!(
        outcome.peer_closed(),
        "the abandonment must keep the peer-closed state it was given"
    );

    let (abandonments, reasons) = abandonment(&snapshotter.snapshot().into_vec());
    assert_eq!(
        abandonments, 1,
        "the drain must record exactly one abandonment"
    );
    assert_eq!(
        reasons,
        ["idle_grace_expired"],
        "the grace, not the shutdown token, ended this write"
    );

    let warnings = warnings_for(&mut log, connection_id);
    assert_eq!(
        warnings.len(),
        1,
        "exactly one abandonment warning must be captured; captured: {warnings:#?}"
    );
    let warning = warnings
        .first()
        .expect("the single captured warning must be inspectable");
    assert_warning_names(warning, peer);
}
