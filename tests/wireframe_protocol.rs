#![cfg(not(loom))]
//! Integration tests for the `WireframeProtocol` trait.
//!
//! These tests ensure that protocol implementations integrate correctly with
//! [`WireframeApp`] and [`ConnectionActor`]. They verify that hooks are invoked
//! with the expected connection context and that frame mutations occur as
//! intended.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use futures::stream;
use rstest::{fixture, rstest};
use tokio_util::sync::CancellationToken;
use wireframe::{
    ConnectionContext,
    WireframeProtocol,
    app::Envelope,
    connection::ConnectionActor,
    push::PushQueues,
    serializer::BincodeSerializer,
};

type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

#[fixture]
fn queues() -> (PushQueues<Vec<u8>>, wireframe::push::PushHandle<Vec<u8>>) {
    PushQueues::<Vec<u8>>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .unlimited()
        .build()
        .expect("failed to build PushQueues")
}

struct TestProtocol {
    counter: Arc<AtomicUsize>,
}

impl WireframeProtocol for TestProtocol {
    type Frame = Vec<u8>;
    type ProtocolError = ();

    fn on_connection_setup(
        &self,
        _handle: wireframe::push::PushHandle<Self::Frame>,
        _ctx: &mut ConnectionContext,
    ) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }

    fn before_send(&self, frame: &mut Self::Frame, _ctx: &mut ConnectionContext) { frame.push(1); }

    fn on_command_end(&self, _ctx: &mut ConnectionContext) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }
}

#[rstest]
#[tokio::test]
async fn builder_produces_protocol_hooks(
    queues: (PushQueues<Vec<u8>>, wireframe::push::PushHandle<Vec<u8>>),
) {
    let counter = Arc::new(AtomicUsize::new(0));
    let protocol = TestProtocol {
        counter: counter.clone(),
    };
    let app = TestApp::new()
        .expect("failed to create app")
        .with_protocol(protocol);
    let mut hooks = app.protocol_hooks();
    let (_queues, handle) = queues;
    hooks.on_connection_setup(handle, &mut ConnectionContext);

    let mut frame = vec![1u8];
    hooks.before_send(&mut frame, &mut ConnectionContext);
    hooks.on_command_end(&mut ConnectionContext);

    assert_eq!(frame, vec![1, 1]);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[rstest]
#[tokio::test]
async fn connection_actor_uses_protocol_from_builder(
    queues: (PushQueues<Vec<u8>>, wireframe::push::PushHandle<Vec<u8>>),
) {
    let counter = Arc::new(AtomicUsize::new(0));
    let protocol = TestProtocol {
        counter: counter.clone(),
    };
    let app = TestApp::new()
        .expect("failed to create app")
        .with_protocol(protocol);

    let hooks = app.protocol_hooks();
    let (queues, handle) = queues;
    handle
        .push_high_priority(vec![1])
        .await
        .expect("push failed");
    let stream = stream::iter(vec![Ok(vec![2u8])]);
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::with_hooks(
        queues,
        handle,
        Some(Box::pin(stream)),
        CancellationToken::new(),
        hooks,
    );
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![vec![1, 1], vec![2, 1]]);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}
