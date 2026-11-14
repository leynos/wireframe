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
use tokio_util::sync::CancellationToken;

use super::{PushBatch, PushTransport, deliver_pushes, drain_queued};
use crate::{
    app::{Envelope, codec_driver::FramePipeline},
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

/// Sink that accepts each write after [`PACING`], recording what it wrote.
///
/// The first write is accepted at once; every later one waits a full pacing
/// interval. That models a peer that keeps reading slowly, and makes the batch
/// outlast one idle grace without any single wait reaching it.
struct PacedSink {
    /// Frames accepted so far, in write order.
    accepted: Arc<Mutex<Vec<Bytes>>>,
    /// Pause owed to the next write, armed once a write has happened.
    delay: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl PacedSink {
    /// Share the sink's record of accepted frames with the test.
    fn new() -> (Self, Arc<Mutex<Vec<Bytes>>>) {
        let accepted = Arc::new(Mutex::new(Vec::new()));
        let sink = Self {
            accepted: Arc::clone(&accepted),
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
        self.delay = Some(Box::pin(tokio::time::sleep(PACING)));
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
