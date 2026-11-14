//! Abandoning a write the peer never accepts.

use std::{
    future::Future,
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use tokio_util::sync::CancellationToken;

use super::{counting_hooks, grace::ManualGraceTimer, test_actor};
use crate::{
    app::Envelope,
    connection::{ConnectionActor, ConnectionChannels, FrameSink, ResponseOutcome},
    hooks::ProtocolHooks,
    push::{PushConfigError, PushQueues},
    response::{FrameStream, Response},
};

/// Sink whose writes never complete, modelling a peer that stopped reading.
struct StalledSink;

impl FrameSink<Envelope> for StalledSink {
    fn send_frame(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        std::future::pending()
    }
}

/// Build an actor whose shutdown token the test controls.
fn test_actor_with_shutdown(
    shutdown: CancellationToken,
    hooks: ProtocolHooks<Envelope, ()>,
) -> Result<ConnectionActor<Envelope, ()>, PushConfigError> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    Ok(ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle),
        None,
        shutdown,
        hooks,
    ))
}

/// A response with one frame ready to write.
fn one_frame_response() -> Response<Envelope, ()> {
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![Ok(
        Envelope::new(1, None, vec![1]),
    )]));
    Response::Stream(stream)
}

/// A pending write must be abandoned when the server shuts down.
///
/// A peer that stops reading applies back-pressure that can hold a write open
/// indefinitely. Without bounding it, a graceful server drain would wait for a
/// peer that may never read again.
#[tokio::test]
async fn shutdown_abandons_a_write_the_peer_never_accepts() {
    let shutdown = CancellationToken::new();
    let mut actor = test_actor_with_shutdown(shutdown.clone(), ProtocolHooks::default())
        .expect("actor should build");
    let mut sink = StalledSink;

    let outcome = {
        let mut driving =
            std::pin::pin!(actor.install_response(one_frame_response(), Some(7), &mut sink));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the write must be pending before shutdown is requested"
        );

        shutdown.cancel();
        driving
            .await
            .expect("an abandoned response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
}

/// The idle grace must also bound a write the peer never accepts.
///
/// The grace bounds the wait for the next event, so a write left outside it
/// would still let a vanished peer pin the connection and the server drain.
/// Both orderings matter: the peer may already be unobservable when the write
/// starts, or observation may end while the write is still pending. The second
/// is what a peer that closes its input half and then stops reading looks like.
#[rstest::rstest]
#[case::peer_already_closed(true)]
#[case::peer_closes_during_the_write(false)]
#[tokio::test]
async fn the_grace_abandons_a_write_the_peer_never_accepts(#[case] close_peer_before_write: bool) {
    let fired = CancellationToken::new();
    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_grace_timer(Arc::new(ManualGraceTimer {
        fired: fired.clone(),
    }));
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let mut sink = StalledSink;

    if close_peer_before_write {
        peer_closed.cancel();
    }
    let outcome = {
        let mut driving =
            std::pin::pin!(actor.install_response(one_frame_response(), Some(7), &mut sink));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the write must be pending before the grace fires"
        );

        if !close_peer_before_write {
            peer_closed.cancel();
            assert!(
                futures::poll!(driving.as_mut()).is_pending(),
                "a write pending at the half-close must survive the grace itself"
            );
        }

        fired.cancel();
        driving
            .await
            .expect("an abandoned response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
}

/// A single-frame response write must also be abandonable.
///
/// `Single` and `Vec` responses look small, but one frame is enough to block
/// on a peer that stopped reading, so their write is bounded like a stream
/// write. A cancelled write reports no completion, because the connection is
/// closing and the response was not fully delivered.
#[tokio::test]
async fn shutdown_abandons_a_single_frame_response_write() {
    let shutdown = CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor_with_shutdown(shutdown.clone(), counting_hooks(Arc::clone(&calls)))
        .expect("actor should build");
    let mut sink = StalledSink;

    let outcome = {
        let mut driving = std::pin::pin!(actor.install_response(
            Response::Single(Envelope::new(1, None, vec![1])),
            Some(7),
            &mut sink,
        ));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the write must be pending before shutdown is requested"
        );

        shutdown.cancel();
        driving
            .await
            .expect("an abandoned response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "an abandoned response must not run the completion hook"
    );
}
