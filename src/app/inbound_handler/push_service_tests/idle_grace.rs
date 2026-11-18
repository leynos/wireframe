//! Counted and warned idle-grace abandonment of a push write.
//!
//! The graceful drain bounds each write separately, so a peer that stopped
//! reading must be reported as the idle grace expiring rather than as a server
//! stop. These cases hold the shutdown token open so only the grace can end the
//! write, and then read what the operator would see.

use super::*;

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
    use std::net::SocketAddr;

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use wireframe_testing::logger;

    use crate::{
        app::PushWriteDiagnostics,
        metrics::PUSH_WRITES_ABANDONED,
        push::PushPriority,
        session::ConnectionId,
    };

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

    let samples = snapshotter.snapshot().into_vec();
    let (key, _, _, value) = samples
        .iter()
        .find(|(key, ..)| key.key().name() == PUSH_WRITES_ABANDONED)
        .expect("the idle-grace abandonment must be counted");
    assert!(
        matches!(value, DebugValue::Counter(1)),
        "the drain must record exactly one abandonment"
    );
    let reasons: Vec<_> = key
        .key()
        .labels()
        .filter(|label| label.key() == "reason")
        .map(::metrics::Label::value)
        .collect();
    assert_eq!(
        reasons,
        ["idle_grace_expired"],
        "the grace, not the shutdown token, ended this write"
    );

    // The log buffer is process-wide, so only records naming this connection
    // count as this test's abandonment.
    let warnings: Vec<String> = std::iter::from_fn(|| log.pop())
        .map(|record| record.args().to_owned())
        .filter(|message| message.contains(&connection_id.to_string()))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "exactly one abandonment warning must be captured; captured: {warnings:#?}"
    );
    let warning = warnings
        .first()
        .expect("the single captured warning must be inspectable");
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
