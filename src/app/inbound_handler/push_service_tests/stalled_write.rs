//! Abandonment coverage for a push write that cannot complete.
//!
//! A write to a peer that has stopped reading must keep expiring the inbound
//! state the read loop would otherwise purge, and the shutdown path that
//! abandons a fast-path write must record the abandonment that the bounded path
//! records. Both cases need a transport whose write never completes, so they
//! live beside the transport rig rather than with the delivery tests.

use std::io;

use bytes::Bytes;
use futures::stream;
use tokio_util::sync::CancellationToken;

use super::{
    PushBatch,
    READ_TIMEOUT_MS,
    empty_inbound,
    transport::{TransportRig, blocked_transport},
};
use crate::{
    app::inbound_handler::{
        frames::{InboundExpiry, buffered_assemblies, incomplete_inbound},
        push_write::write_frame,
    },
    connection::PEER_CLOSED_RESPONSE_GRACE,
};

/// Read timeout the stalled-write case uses, shorter than the idle grace.
const STALLED_READ_TIMEOUT_MS: u64 = 1_000;

/// A stalled push write keeps the inbound expiry running.
///
/// While the write is pending the loop does not poll the reader, so the read
/// deadline would otherwise never be observed and a partial assembly could
/// outlive its timeout. The purge must not cancel or restart the frame: the
/// write keeps its place on the wire and is ended only by the idle grace.
#[rstest::rstest]
#[tokio::test(start_paused = true)]
async fn a_stalled_push_write_purges_expired_inbound_state(
    #[from(blocked_transport)] mut harness: TransportRig,
) {
    use crate::push::PushPriority;

    let shutdown = CancellationToken::new();
    let peer_eof = CancellationToken::new();
    let mut inbound = incomplete_inbound::<Bytes>();
    let mut reader = stream::pending::<io::Result<Bytes>>();
    let mut batch = PushBatch {
        frames: Vec::new(),
        priority: Some(PushPriority::High),
        reader: &mut reader,
        pending_frame: &mut inbound.pending,
        expiry: InboundExpiry::new(
            &mut inbound.message_assembly,
            &mut inbound.read_deadline,
            STALLED_READ_TIMEOUT_MS,
        ),
        peer_eof: &peer_eof,
        shutdown: &shutdown,
    };

    // The peer closed its write half, so the write is bounded by the grace from
    // its first poll. The read deadline is shorter still, so the purge happens
    // several times before the grace abandons the frame.
    let written = tokio::time::timeout(
        PEER_CLOSED_RESPONSE_GRACE * 2,
        write_frame(
            &mut harness.transport(),
            Bytes::from_static(b"stalled"),
            &mut batch,
            true,
        ),
    )
    .await
    .expect("the write must be bounded")
    .expect("an idle-grace abandonment is not an error");
    drop(batch);

    assert!(
        !written.may_continue(),
        "the grace, not a purge, must end the stalled write"
    );
    assert!(
        written.peer_closed(),
        "the abandonment must keep the peer-closed state it was given"
    );
    assert_eq!(
        buffered_assemblies(&inbound),
        Some(0),
        "the stalled write must not postpone the inbound expiry"
    );
}

/// A shutdown that abandons a fast-path write is counted, not just returned.
///
/// The first select prefers a ready write, so its shutdown arm handles only a
/// write that is still pending. That arm must record the abandonment like the
/// bounded path does; otherwise `reason="shutdown"` under-counts the frames a
/// stop actually dropped.
#[rstest::rstest]
#[tokio::test]
async fn a_shutdown_abandoned_fast_path_write_is_counted(
    #[from(blocked_transport)] mut harness: TransportRig,
) {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use crate::{metrics::PUSH_WRITES_ABANDONED, push::PushPriority};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let peer_eof = CancellationToken::new();
    let mut inbound = empty_inbound();
    let mut reader = stream::pending::<io::Result<Bytes>>();
    let frame = Bytes::from_static(b"abandoned");
    let mut batch = PushBatch {
        frames: Vec::new(),
        // A single lane, so the diagnostics would name `high` in a warning.
        // The counter keeps its fixed `reason` label and grows no priority
        // label from it.
        priority: Some(PushPriority::High),
        reader: &mut reader,
        pending_frame: &mut inbound.pending,
        expiry: InboundExpiry::new(
            &mut inbound.message_assembly,
            &mut inbound.read_deadline,
            READ_TIMEOUT_MS,
        ),
        peer_eof: &peer_eof,
        shutdown: &shutdown,
    };

    let outcome = {
        let _recorder = ::metrics::set_default_local_recorder(&recorder);
        write_frame(&mut harness.transport(), frame, &mut batch, false).await
    }
    .expect("a shutdown abandonment is not an error");
    assert!(!outcome.may_continue(), "the connection must stop");
    assert!(
        !outcome.peer_closed(),
        "the peer did not close its write half"
    );

    let samples = snapshotter.snapshot().into_vec();
    let (key, _, _, value) = samples
        .iter()
        .find(|(key, ..)| key.key().name() == PUSH_WRITES_ABANDONED)
        .expect("the shutdown abandonment must be counted");
    assert!(
        matches!(value, DebugValue::Counter(1)),
        "the fast path must record exactly one abandonment"
    );
    assert!(
        key.key().labels().count() == 1
            && key
                .key()
                .labels()
                .any(|label| label.key() == "reason" && label.value() == "shutdown"),
        "the counter keeps its single reason label and adds no priority label"
    );
}
