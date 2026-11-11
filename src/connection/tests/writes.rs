//! Abandoning a write the peer never accepts.

use std::{future::Future, io, sync::Arc};

use tokio_util::sync::CancellationToken;

use super::{grace::ManualGraceTimer, test_actor};
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
) -> Result<ConnectionActor<Envelope, ()>, PushConfigError> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    Ok(ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle),
        None,
        shutdown,
        ProtocolHooks::default(),
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
    let mut actor = test_actor_with_shutdown(shutdown.clone()).expect("actor should build");
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
#[tokio::test]
async fn the_grace_abandons_a_write_the_peer_never_accepts() {
    let fired = CancellationToken::new();
    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_grace_timer(Arc::new(ManualGraceTimer {
        fired: fired.clone(),
    }));
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let mut sink = StalledSink;

    peer_closed.cancel();
    let outcome = {
        let mut driving =
            std::pin::pin!(actor.install_response(one_frame_response(), Some(7), &mut sink));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "an unfired grace must not abandon a pending write"
        );

        fired.cancel();
        driving
            .await
            .expect("an abandoned response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
}
