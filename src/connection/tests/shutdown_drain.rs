//! Draining a response across shutdown, and abandoning it on request.
//!
//! ADR-014 requires a server connection to keep draining a response whose peer
//! is still reading after `stop()`, so the stop token bounds only writes that
//! cannot progress. The actor therefore lets its owner choose whether a
//! cancelled shutdown token ends the response drive, and the direct actor API
//! keeps the abandon behaviour its own tests pin.

use std::{future::Future, io, time::Duration};

use tokio_util::sync::CancellationToken;

use super::{ConnectionActor, ConnectionChannels, UntrackedActorInit, test_actor_with_shutdown};
use crate::{
    app::Envelope,
    connection::{FrameSink, PushSink, ResponseOutcome},
    hooks::ProtocolHooks,
    push::{PushError, PushQueues},
    response::{FrameStream, Response},
};

/// Sink that records each payload byte and can request shutdown after the
/// first accepted write.
///
/// Cancelling the token inside the write models a server `stop()` that arrives
/// while the response is still producing, without depending on scheduling.
/// Both frame kinds take the token, because a queued push can be written before
/// the response frames produced with it.
struct RecordingSink {
    /// Token cancelled once the first write has been accepted, when set.
    shutdown: Option<CancellationToken>,
    /// Payload byte of every accepted frame, in order.
    accepted: Vec<u8>,
}

impl RecordingSink {
    /// Record one accepted write, requesting shutdown on the first.
    fn accept(&mut self, frame: &Envelope) {
        if let Some(byte) = frame.payload_bytes().first() {
            self.accepted.push(*byte);
        }
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.cancel();
        }
    }
}

impl FrameSink<Envelope> for RecordingSink {
    fn send_frame(&mut self, frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        self.accept(&frame);
        std::future::ready(Ok(()))
    }
}

impl PushSink<Envelope> for RecordingSink {
    fn send_push(&mut self, frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        self.accept(&frame);
        std::future::ready(Ok(()))
    }
}

/// A three-frame response whose payload byte numbers each frame.
fn three_frame_response() -> Response<Envelope, ()> {
    let frames = (1..=3)
        .map(|number| Ok(Envelope::new(1, None, vec![number])))
        .collect::<Vec<_>>();
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(frames));
    Response::Stream(stream)
}

/// Shutdown either drains the response or abandons it, as the owner chose.
///
/// The stop request arrives after the first frame reached the sink, while the
/// stream still has two frames to produce. A connection that drains writes all
/// three and reports completion; the direct actor API stops after the frame it
/// had already written.
#[rstest::rstest]
#[case::drains_across_shutdown(false, 3, ResponseOutcome::Completed)]
#[case::abandons_on_shutdown(true, 1, ResponseOutcome::Cancelled)]
#[tokio::test]
async fn shutdown_either_drains_or_abandons_a_stream(
    #[case] abandon_on_shutdown: bool,
    #[case] expected_frames: usize,
    #[case] expected_outcome: ResponseOutcome,
) {
    let shutdown = CancellationToken::new();
    let mut actor = test_actor_with_shutdown(shutdown.clone(), ProtocolHooks::default())
        .expect("the actor should build");
    actor.set_abandon_response_on_shutdown(abandon_on_shutdown);
    let mut sink = RecordingSink {
        shutdown: Some(shutdown),
        accepted: Vec::new(),
    };

    let outcome = actor
        .install_response(three_frame_response(), Some(7), &mut sink)
        .await
        .expect("a bounded response is not an error");

    assert_eq!(outcome, expected_outcome);
    assert_eq!(
        sink.accepted.len(),
        expected_frames,
        "unexpected frames reached the sink: {:?}",
        sink.accepted
    );
}

/// A response that yields nothing, so only the idle grace can end it.
fn stalled_response() -> Response<Envelope, ()> {
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::pending());
    Response::Stream(stream)
}

/// A stop that starts the bounded drain drops pushes rather than flushing them.
///
/// ADR-014 keeps the response draining across `stop()`, but the connection is
/// still ending. The documented shutdown rule is that a frame waiting in a push
/// queue at that moment is dropped, and a producer must not be able to renew
/// the drain's idle grace with a fresh push. The frame queued before the stop
/// therefore never reaches the sink, and a push after it is refused outright.
#[tokio::test(start_paused = true)]
async fn shutdown_that_starts_the_bounded_drain_drops_queued_pushes() {
    let shutdown = CancellationToken::new();
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("the push queues should build");
    let mut actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle.clone()),
        response: None,
        shutdown: shutdown.clone(),
        hooks: ProtocolHooks::default(),
        ctx: crate::hooks::ConnectionContext::default(),
    });
    // Server semantics: a stop drains the response rather than abandoning it.
    actor.set_abandon_response_on_shutdown(false);
    let mut sink = RecordingSink {
        shutdown: None,
        accepted: Vec::new(),
    };
    handle
        .push_high_priority(Envelope::new(1, None, vec![9]))
        .await
        .expect("the push is accepted before the stop");
    shutdown.cancel();

    let outcome = actor
        .install_response(stalled_response(), Some(1), &mut sink)
        .await
        .expect("a bounded response is not an error");

    assert_eq!(
        outcome,
        ResponseOutcome::Cancelled,
        "the idle grace must end a stalled response"
    );
    assert!(
        !sink.accepted.contains(&9),
        "a push queued when the stop arrived must be dropped, not flushed"
    );
    assert_eq!(
        handle
            .push_high_priority(Envelope::new(1, None, vec![10]))
            .await,
        Err(PushError::Closed),
        "a stopping connection must refuse a new push rather than queue it"
    );
}

/// A stop that arrives during a peer-close drain still closes the push lanes.
///
/// The drain is entered because the peer half-closed, not because of a stop, so
/// it keeps servicing the push lanes. A stop observed after that must still
/// close them: the frame queued at that moment is dropped, a later push is
/// refused, and the response itself keeps draining until the idle grace ends it.
/// The sink requests the stop on its first accepted write, so the drain only
/// observes it after the push it had already started has been written.
#[tokio::test(start_paused = true)]
async fn a_stop_during_a_peer_close_drain_discards_the_push_lanes() {
    let shutdown = CancellationToken::new();
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(2)
        .low_capacity(1)
        .build()
        .expect("the push queues should build");
    let mut actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle.clone()),
        response: None,
        shutdown: shutdown.clone(),
        hooks: ProtocolHooks::default(),
        ctx: crate::hooks::ConnectionContext::default(),
    });
    // Server semantics: a stop drains the response rather than abandoning it.
    actor.set_abandon_response_on_shutdown(false);
    actor.set_peer_closed_grace(Duration::from_secs(5));
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let mut sink = RecordingSink {
        shutdown: Some(shutdown),
        accepted: Vec::new(),
    };
    for byte in [9, 10] {
        handle
            .push_high_priority(Envelope::new(1, None, vec![byte]))
            .await
            .expect("the push is accepted before the stop");
    }
    peer_closed.cancel();

    let outcome = actor
        .install_response(stalled_response(), Some(1), &mut sink)
        .await
        .expect("a bounded response is not an error");

    assert_eq!(
        sink.accepted.first(),
        Some(&9),
        "the push written before the stop was accepted must reach the sink"
    );
    assert!(
        !sink.accepted.contains(&10),
        "a push queued when the stop arrived must be dropped, not flushed"
    );
    assert_eq!(
        outcome,
        ResponseOutcome::Cancelled,
        "the idle grace must end a stalled response"
    );
    assert_eq!(
        handle
            .push_high_priority(Envelope::new(1, None, vec![11]))
            .await,
        Err(PushError::Closed),
        "a stopping connection must refuse a new push rather than queue it"
    );
}
