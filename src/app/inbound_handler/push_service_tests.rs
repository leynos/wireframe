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
use transport::{TransportRig, immediate_transport, paced_transport};

use super::{
    super::frames::{InboundExpiry, InboundFrames},
    PushBatch,
    deliver_pushes,
    drain_queued,
};
use crate::{
    app::{Envelope, codec_driver::FramePipeline},
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

#[cfg(feature = "metrics")]
#[path = "push_service_tests/stalled_write.rs"]
mod stalled_write;

#[path = "push_service_tests/transport.rs"]
mod transport;

/// Capacity of each push lane, enough for every frame these tests queue.
const LANE_CAPACITY: usize = 4;

/// Read timeout used by tests that do not exercise the inbound expiry.
const READ_TIMEOUT_MS: u64 = 30_000;

/// Inbound read state with nothing buffered and no silence under way.
fn empty_inbound() -> InboundFrames<Bytes> {
    InboundFrames {
        pending: None,
        pipeline: FramePipeline::new(None),
        message_assembly: None,
        read_deadline: None,
    }
}

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
/// Keeping them alive keeps the actor's matching lane receiver reporting
/// `Empty` rather than exhausted, which is the producer that could otherwise
/// pin the end-of-stream drain.
struct HeldReservations {
    /// Reservation on the high-priority lane.
    _high: mpsc::OwnedPermit<Bytes>,
    /// Reservation on the low-priority lane.
    _low: mpsc::OwnedPermit<Bytes>,
}

/// The frame a reserved actor accepted before the peer closed.
const ACCEPTED_PAYLOAD: &[u8] = b"accepted";

/// Build an actor whose lanes hold both an accepted frame and a reservation.
///
/// The accepted frame models a producer whose push already returned `Ok`; the
/// reservation models a producer that has taken capacity and has not sent. The
/// lanes are created here rather than by the queue builder because only an
/// `mpsc::Sender` can take a reservation, and the handle comes from a separate
/// pair that nothing pushes through.
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
    high_tx
        .try_send(Bytes::from_static(ACCEPTED_PAYLOAD))
        .expect("a fresh lane has room for the accepted frame");
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
    let mut inbound = empty_inbound();
    let mut reader = stream::pending::<io::Result<Bytes>>();

    let started = tokio::time::Instant::now();
    let may_continue = deliver_pushes(
        &mut harness.transport(),
        &mut actor,
        PushBatch {
            frames: paced_payloads().to_vec(),
            priority: None,
            reader: &mut reader,
            pending_frame: &mut inbound.pending,
            expiry: InboundExpiry::new(
                &mut inbound.message_assembly,
                &mut inbound.read_deadline,
                READ_TIMEOUT_MS,
            ),
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

/// An unsent reservation does not hold up the frames already accepted.
///
/// The lane reports itself exhausted only once every permit taken before the
/// close has been released, so a producer that never sends would stall an
/// awaited collection before its first write. The drain seals the lanes and
/// takes what they already hold instead; the shutdown token is deliberately
/// left uncancelled so nothing else can end the wait.
#[rstest::rstest]
#[tokio::test(start_paused = true)]
async fn an_unsent_reservation_does_not_hold_up_the_drain(
    #[from(immediate_transport)] mut harness: TransportRig,
) {
    let shutdown = CancellationToken::new();
    let (mut actor, reservations) = build_reserved_actor(&shutdown).expect("actor should build");

    tokio::time::timeout(
        Duration::from_secs(1),
        drain_queued(&mut harness.transport(), &mut actor, &shutdown),
    )
    .await
    .expect("the drain must not wait for an unsent reservation")
    .expect("the drain must not fail");

    assert_eq!(
        harness.accepted_frames(),
        vec![Bytes::from_static(ACCEPTED_PAYLOAD)],
        "the frame whose producer was told it was accepted must still be written"
    );
    drop(reservations);
}
