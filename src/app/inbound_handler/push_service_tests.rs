//! Per-frame idle-grace coverage for queued push delivery.
//!
//! A push is bounded once per frame rather than once per batch: a peer that
//! accepts every frame within the idle grace must receive the whole batch even
//! when the batch takes longer than one grace in total. These tests pace each
//! sink write below the grace, so a deadline created once for the batch would
//! deliver only part of it and leave the rest abandoned.

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use futures::{Sink, stream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{PushBatch, PushTransport, deliver_pushes, drain_queued};
use crate::{
    app::{Envelope, PushWriteDiagnostics, codec_driver::FramePipeline},
    codec::LengthDelimitedFrameCodec,
    connection::{
        ConnectionActor,
        ConnectionChannels,
        PEER_CLOSED_RESPONSE_GRACE,
        UntrackedActorInit,
    },
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushConfigError, PushHandle, PushQueues},
    serializer::BincodeSerializer,
};

/// Maximum frame length used by the in-memory codec.
const MAX_FRAME_LENGTH: usize = 2048;
/// Capacity of each push lane, enough for every frame these tests queue.
const LANE_CAPACITY: usize = 4;
/// Pause applied before each sink write after the first.
///
/// Below the idle grace, so every write is accepted in time, but long enough
/// that three frames outlast one grace in total.
const PACING: Duration = Duration::from_secs(20);

/// Sink that records what it wrote and can pace later writes.
///
/// The first write is accepted at once; every later one waits a full pacing
/// interval when one is configured. That models a peer that keeps reading
/// slowly, and makes the batch outlast one idle grace without any single wait
/// reaching it. A sink built with [`PacedSink::immediate`] configures no pause
/// and accepts every write at once.
struct PacedSink {
    /// Frames accepted so far, in write order.
    accepted: Arc<Mutex<Vec<Bytes>>>,
    /// Pause owed to each write after the first, or `None` to accept at once.
    pacing: Option<Duration>,
    /// Pause owed to the next write, armed once a write has happened.
    delay: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl PacedSink {
    /// Share a sink that pauses between writes with the test.
    fn new() -> (Self, Arc<Mutex<Vec<Bytes>>>) { Self::with_pacing(Some(PACING)) }

    /// Share a sink that accepts every write at once with the test.
    fn immediate() -> (Self, Arc<Mutex<Vec<Bytes>>>) { Self::with_pacing(None) }

    /// Build a sink and share its record of accepted frames with the test.
    fn with_pacing(pacing: Option<Duration>) -> (Self, Arc<Mutex<Vec<Bytes>>>) {
        let accepted = Arc::new(Mutex::new(Vec::new()));
        let sink = Self {
            accepted: Arc::clone(&accepted),
            pacing,
            delay: None,
        };
        (sink, accepted)
    }

    /// Record one accepted frame and arm the next write's pause.
    fn accept(&mut self, frame: Bytes) {
        self.accepted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frame);
        self.delay = self
            .pacing
            .map(|pacing| Box::pin(tokio::time::sleep(pacing)));
    }
}

impl Sink<Bytes> for PacedSink {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let is_ready = self
            .delay
            .as_mut()
            .is_none_or(|delay| matches!(delay.as_mut().poll(cx), Poll::Ready(())));
        if !is_ready {
            return Poll::Pending;
        }
        self.delay = None;
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, frame: Bytes) -> Result<(), Self::Error> {
        self.accept(frame);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
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

/// Snapshot the frames a paced sink accepted.
fn accepted_frames(accepted: &Arc<Mutex<Vec<Bytes>>>) -> Vec<Bytes> {
    accepted
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
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
#[tokio::test(start_paused = true)]
async fn a_slow_but_progressing_peer_receives_every_queued_frame() {
    let shutdown = CancellationToken::new();
    let (mut actor, handle) = build_actor(&shutdown).expect("actor should build");
    for payload in paced_payloads() {
        handle
            .push_high_priority(payload)
            .await
            .expect("the lane has room for every payload");
    }

    let serializer = BincodeSerializer;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut pipeline = FramePipeline::<Bytes>::new(None);
    let (mut sink, accepted) = PacedSink::new();
    let mut transport = PushTransport {
        serializer: &serializer,
        codec: &codec,
        sink: &mut sink,
        pipeline: &mut pipeline,
        diagnostics: PushWriteDiagnostics::new(None, None),
    };

    let started = tokio::time::Instant::now();
    drain_queued(&mut transport, &mut actor, &shutdown)
        .await
        .expect("the drain must not fail");
    let elapsed = started.elapsed();

    assert!(
        elapsed > PEER_CLOSED_RESPONSE_GRACE,
        "the batch must outlast one idle grace: {elapsed:?}"
    );
    assert_eq!(
        accepted_frames(&accepted),
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
#[tokio::test(start_paused = true)]
async fn a_closed_batch_renews_the_grace_for_every_frame() {
    let shutdown = CancellationToken::new();
    let (mut actor, _handle) = build_actor(&shutdown).expect("actor should build");

    let serializer = BincodeSerializer;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut pipeline = FramePipeline::<Bytes>::new(None);
    let (mut sink, accepted) = PacedSink::new();
    let mut transport = PushTransport {
        serializer: &serializer,
        codec: &codec,
        sink: &mut sink,
        pipeline: &mut pipeline,
        diagnostics: PushWriteDiagnostics::new(None, None),
    };
    let peer_eof = CancellationToken::new();
    peer_eof.cancel();
    let mut pending_frame = None;
    let mut reader = stream::pending::<io::Result<Bytes>>();

    let started = tokio::time::Instant::now();
    let may_continue = deliver_pushes(
        &mut transport,
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
        accepted_frames(&accepted),
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
#[tokio::test]
async fn a_ready_collection_is_written_after_a_stop() {
    let shutdown = CancellationToken::new();
    let (mut actor, handle) = build_actor(&shutdown).expect("actor should build");
    for payload in paced_payloads() {
        handle
            .push_high_priority(payload)
            .await
            .expect("the lane has room for every payload");
    }
    shutdown.cancel();

    let serializer = BincodeSerializer;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut pipeline = FramePipeline::<Bytes>::new(None);
    let (mut sink, accepted) = PacedSink::immediate();
    let mut transport = PushTransport {
        serializer: &serializer,
        codec: &codec,
        sink: &mut sink,
        pipeline: &mut pipeline,
        diagnostics: PushWriteDiagnostics::new(None, None),
    };

    drain_queued(&mut transport, &mut actor, &shutdown)
        .await
        .expect("the drain must not fail");

    assert_eq!(
        accepted_frames(&accepted),
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
#[tokio::test]
async fn a_stop_abandons_a_collection_that_is_still_pending() {
    let shutdown = CancellationToken::new();
    let (mut actor, reservations) = build_reserved_actor(&shutdown).expect("actor should build");
    shutdown.cancel();

    let serializer = BincodeSerializer;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut pipeline = FramePipeline::<Bytes>::new(None);
    let (mut sink, accepted) = PacedSink::immediate();
    let mut transport = PushTransport {
        serializer: &serializer,
        codec: &codec,
        sink: &mut sink,
        pipeline: &mut pipeline,
        diagnostics: PushWriteDiagnostics::new(None, None),
    };

    tokio::time::timeout(
        Duration::from_secs(1),
        drain_queued(&mut transport, &mut actor, &shutdown),
    )
    .await
    .expect("a pending collection must not outlive the stop")
    .expect("an abandoned drain must not fail");

    assert!(
        accepted_frames(&accepted).is_empty(),
        "an abandoned collection must write nothing"
    );
    drop(reservations);
}
