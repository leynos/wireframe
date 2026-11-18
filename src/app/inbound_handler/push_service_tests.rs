//! Per-frame idle-grace coverage for queued push delivery.
//!
//! A push is bounded once per frame rather than once per batch: a peer that
//! accepts every frame within the idle grace must receive the whole batch even
//! when the batch takes longer than one grace in total. These tests pace each
//! sink write below the grace, so a deadline created once for the batch would
//! deliver only part of it and leave the rest abandoned.

use std::{io, time::Duration};

use bytes::Bytes;
use futures::stream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
#[cfg(feature = "metrics")]
use transport::blocked_transport;
use transport::{TransportRig, immediate_transport, paced_transport};

#[cfg(feature = "metrics")]
use super::super::push_write::write_frame;
use super::{PushBatch, deliver_pushes, drain_queued};
use crate::{
    app::Envelope,
    connection::{
        ConnectionActor,
        ConnectionChannels,
        PEER_CLOSED_RESPONSE_GRACE,
        UntrackedActorInit,
    },
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushConfigError, PushHandle, PushQueues},
};

#[cfg(feature = "metrics")]
#[path = "push_service_tests/idle_grace.rs"]
mod idle_grace;

#[path = "push_service_tests/transport.rs"]
mod transport;

/// Capacity of each push lane, enough for every frame these tests queue.
const LANE_CAPACITY: usize = 4;

/// Untracked actor paired with the handle that keeps its push lanes open.
type ActorFixture = (ConnectionActor<Envelope, (), Bytes>, PushHandle<Bytes>);

/// Build an untracked actor with room for the frames these tests queue.
///
/// # Errors
///
/// Returns [`PushConfigError`] if the lane capacities are rejected.
fn build_actor(shutdown: &CancellationToken) -> Result<ActorFixture, PushConfigError> {
    let (queues, handle) = PushQueues::<Bytes>::builder()
        .high_capacity(LANE_CAPACITY)
        .low_capacity(LANE_CAPACITY)
        .build()?;
    let actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle.clone()),
        response: None,
        shutdown: shutdown.clone(),
        hooks: ProtocolHooks::default(),
        ctx: ConnectionContext::default(),
    });
    Ok((actor, handle))
}

/// Reservations held by a sender that has not sent what it reserved.
///
/// Keeping them alive keeps the actor's matching lane receiver pending even
/// after the drain closes it, which is the producer that could otherwise pin
/// the end-of-stream drain.
struct HeldReservations {
    /// Reservation on the high-priority lane.
    _high: mpsc::OwnedPermit<Bytes>,
    /// Reservation on the low-priority lane.
    _low: mpsc::OwnedPermit<Bytes>,
}

/// Build an actor whose push lanes still have reservations outstanding.
///
/// The lanes are chosen by this helper rather than by the queue builder so a
/// reservation can be taken before the actor owns the receivers. The handle
/// comes from a separate pair, which is unused here: no frame is ever pushed
/// through it.
///
/// # Errors
///
/// Returns [`PushConfigError`] if the lane capacities are rejected.
fn build_reserved_actor(
    shutdown: &CancellationToken,
) -> Result<(ConnectionActor<Envelope, (), Bytes>, HeldReservations), PushConfigError> {
    let (_unused_queues, handle) = PushQueues::<Bytes>::builder()
        .high_capacity(LANE_CAPACITY)
        .low_capacity(LANE_CAPACITY)
        .build()?;
    let (high_tx, high_priority_rx) = mpsc::channel(LANE_CAPACITY);
    let (low_tx, low_priority_rx) = mpsc::channel(LANE_CAPACITY);
    let reservations = HeldReservations {
        _high: high_tx
            .try_reserve_owned()
            .expect("a fresh lane has room for a reservation"),
        _low: low_tx
            .try_reserve_owned()
            .expect("a fresh lane has room for a reservation"),
    };
    let actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(
            PushQueues {
                high_priority_rx,
                low_priority_rx,
            },
            handle,
        ),
        response: None,
        shutdown: shutdown.clone(),
        hooks: ProtocolHooks::default(),
        ctx: ConnectionContext::default(),
    });
    Ok((actor, reservations))
}

/// Three payloads whose total delivery time outlasts one idle grace.
fn paced_payloads() -> [Bytes; 3] {
    [
        Bytes::from_static(b"one"),
        Bytes::from_static(b"two"),
        Bytes::from_static(b"three"),
    ]
}

/// Every queued frame is drained when the peer keeps accepting within grace.
///
/// The drain is only started after the peer closed its write half, so each
/// write is bounded by the idle grace from its first poll. A single deadline
/// for the drain would expire while the second frame is still being accepted,
/// so receiving all three proves the grace restarts per frame.
#[rstest::rstest]
#[tokio::test(start_paused = true)]
async fn a_slow_but_progressing_peer_receives_every_queued_frame(
    #[from(paced_transport)] mut harness: TransportRig,
) {
    let shutdown = CancellationToken::new();
    let (mut actor, handle) = build_actor(&shutdown).expect("actor should build");
    for payload in paced_payloads() {
        handle
            .push_high_priority(payload)
            .await
            .expect("the lane has room for every payload");
    }

    let started = tokio::time::Instant::now();
    drain_queued(&mut harness.transport(), &mut actor, &shutdown)
        .await
        .expect("the drain must not fail");
    let elapsed = started.elapsed();

    assert!(
        elapsed > PEER_CLOSED_RESPONSE_GRACE,
        "the batch must outlast one idle grace: {elapsed:?}"
    );
    assert_eq!(
        harness.accepted_frames(),
        paced_payloads().to_vec(),
        "a peer accepting each frame within the grace must receive the whole batch"
    );
}

/// Every frame in an already closed batch is written under its own grace.
///
/// The peer closed its write half before the batch was written, so no further
/// observation is possible and `deliver_pushes` bounds the batch directly.
/// Receiving all three frames proves that bound is renewed rather than applied
/// once to the whole batch.
#[rstest::rstest]
#[tokio::test(start_paused = true)]
async fn a_closed_batch_renews_the_grace_for_every_frame(
    #[from(paced_transport)] mut harness: TransportRig,
) {
    let shutdown = CancellationToken::new();
    let (mut actor, _handle) = build_actor(&shutdown).expect("actor should build");
    let peer_eof = CancellationToken::new();
    peer_eof.cancel();
    let mut pending_frame = None;
    let mut reader = stream::pending::<io::Result<Bytes>>();

    let started = tokio::time::Instant::now();
    let may_continue = deliver_pushes(
        &mut harness.transport(),
        &mut actor,
        PushBatch {
            frames: paced_payloads().to_vec(),
            priority: None,
            reader: &mut reader,
            pending_frame: &mut pending_frame,
            peer_eof: &peer_eof,
            shutdown: &shutdown,
        },
    )
    .await
    .expect("the write must not fail");
    let elapsed = started.elapsed();

    assert!(!may_continue, "a closed peer must not keep serving");
    assert!(
        elapsed > PEER_CLOSED_RESPONSE_GRACE,
        "the batch must outlast one idle grace: {elapsed:?}"
    );
    assert_eq!(
        harness.accepted_frames(),
        paced_payloads().to_vec(),
        "a closed peer accepting each frame within the grace must receive the whole batch"
    );
}

/// A stop does not discard a collection that is already ready.
///
/// The frames were queued before the stop and the lanes report themselves
/// exhausted at once, so no write is pending when the stop is observed. ADR 014
/// limits abandonment to a write that is still pending, so the ready batch is
/// written even though the token is already cancelled.
#[rstest::rstest]
#[tokio::test]
async fn a_ready_collection_is_written_after_a_stop(
    #[from(immediate_transport)] mut harness: TransportRig,
) {
    let shutdown = CancellationToken::new();
    let (mut actor, handle) = build_actor(&shutdown).expect("actor should build");
    for payload in paced_payloads() {
        handle
            .push_high_priority(payload)
            .await
            .expect("the lane has room for every payload");
    }
    shutdown.cancel();

    drain_queued(&mut harness.transport(), &mut actor, &shutdown)
        .await
        .expect("the drain must not fail");

    assert_eq!(
        harness.accepted_frames(),
        paced_payloads().to_vec(),
        "a ready collection must be written even after a stop"
    );
}

/// A stop abandons a collection that is still waiting on a producer.
///
/// The lane holds a reservation that was never sent, so it reports itself
/// exhausted only once that permit is released. The stop must end the drain
/// without waiting for it, because a producer that never releases one would
/// otherwise pin the connection, its registry entry, and the server drain.
#[rstest::rstest]
#[tokio::test]
async fn a_stop_abandons_a_collection_that_is_still_pending(
    #[from(immediate_transport)] mut harness: TransportRig,
) {
    let shutdown = CancellationToken::new();
    let (mut actor, reservations) = build_reserved_actor(&shutdown).expect("actor should build");
    shutdown.cancel();

    tokio::time::timeout(
        Duration::from_secs(1),
        drain_queued(&mut harness.transport(), &mut actor, &shutdown),
    )
    .await
    .expect("a pending collection must not outlive the stop")
    .expect("an abandoned drain must not fail");

    assert!(
        harness.accepted_frames().is_empty(),
        "an abandoned collection must write nothing"
    );
    drop(reservations);
}

/// A shutdown that abandons a fast-path write is counted, not just returned.
///
/// The first select prefers a ready write, so its shutdown arm handles only a
/// write that is still pending. That arm must record the abandonment like the
/// bounded path does; otherwise `reason="shutdown"` under-counts the frames a
/// stop actually dropped.
#[cfg(feature = "metrics")]
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
    let mut pending_frame = None;
    let mut reader = stream::pending::<io::Result<Bytes>>();
    let frame = Bytes::from_static(b"abandoned");
    let mut batch = PushBatch {
        frames: Vec::new(),
        // A single lane, so the diagnostics would name `high` in a warning.
        // The counter keeps its fixed `reason` label and grows no priority
        // label from it.
        priority: Some(PushPriority::High),
        reader: &mut reader,
        pending_frame: &mut pending_frame,
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
