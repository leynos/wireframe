//! Tests for connection lifecycle callbacks.
//!
//! They check setup, teardown, and state propagation through helper utilities.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::BytesMut;
use wireframe::{
    app::{Envelope, Packet, PacketParts, WireframeApp},
    frame::{FrameProcessor, LengthPrefixedProcessor},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{processor, run_app, run_with_duplex_server};

fn call_counting_callback<R, A>(
    counter: &Arc<AtomicUsize>,
    result: R,
) -> impl Fn(A) -> Pin<Box<dyn Future<Output = R> + Send>> + Clone + 'static
where
    A: Send + 'static,
    R: Clone + Send + 'static,
{
    let counter = counter.clone();
    move |_| {
        let counter = counter.clone();
        let result = result.clone();
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            result
        })
    }
}

fn wireframe_app_with_lifecycle_callbacks<E>(
    setup: &Arc<AtomicUsize>,
    teardown: &Arc<AtomicUsize>,
    state: u32,
) -> WireframeApp<BincodeSerializer, u32, E>
where
    E: Packet,
{
    let setup_cb = call_counting_callback(setup, state);
    let teardown_cb = call_counting_callback(teardown, ());

    WireframeApp::<_, _, E>::new()
        .expect("failed to create app")
        .on_connection_setup(move || setup_cb(()))
        .expect("setup callback")
        .on_connection_teardown(teardown_cb)
        .expect("teardown callback")
}

#[tokio::test]
async fn setup_and_teardown_callbacks_run() {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));

    let app = wireframe_app_with_lifecycle_callbacks::<Envelope>(&setup_count, &teardown_count, 42);

    run_with_duplex_server(app).await;

    assert_eq!(setup_count.load(Ordering::SeqCst), 1);
    assert_eq!(teardown_count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn setup_without_teardown_runs() {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let cb = call_counting_callback(&setup_count, ());

    let app = WireframeApp::<_, _, Envelope>::new()
        .expect("failed to create app")
        .on_connection_setup(move || cb(()))
        .expect("setup callback");

    run_with_duplex_server(app).await;

    assert_eq!(setup_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn teardown_without_setup_does_not_run() {
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let cb = call_counting_callback(&teardown_count, ());

    let app = WireframeApp::<_, _, Envelope>::new()
        .expect("failed to create app")
        .on_connection_teardown(cb)
        .expect("teardown callback");

    run_with_duplex_server(app).await;

    assert_eq!(teardown_count.load(Ordering::SeqCst), 0);
}

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct StateEnvelope {
    id: u32,
    correlation_id: Option<u64>,
    msg: Vec<u8>,
}

impl Packet for StateEnvelope {
    fn id(&self) -> u32 { self.id }

    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn into_parts(self) -> PacketParts {
        PacketParts {
            id: self.id,
            correlation_id: self.correlation_id,
            msg: self.msg,
        }
    }

    fn from_parts(parts: PacketParts) -> Self {
        Self {
            id: parts.id,
            correlation_id: parts.correlation_id,
            msg: parts.msg,
        }
    }
}

#[tokio::test]
async fn helpers_propagate_connection_state() {
    let setup = Arc::new(AtomicUsize::new(0));
    let teardown = Arc::new(AtomicUsize::new(0));

    let app = wireframe_app_with_lifecycle_callbacks::<StateEnvelope>(&setup, &teardown, 7)
        .frame_processor(processor())
        .route(1, Arc::new(|_: &StateEnvelope| Box::pin(async {})))
        .expect("route registration failed");

    let env = StateEnvelope {
        id: 1,
        correlation_id: Some(0),
        msg: vec![1],
    };
    let bytes = BincodeSerializer
        .serialize(&env)
        .expect("failed to serialise envelope");
    let mut frame = BytesMut::new();
    LengthPrefixedProcessor::default()
        .encode(&bytes, &mut frame)
        .expect("encode should succeed");

    let out = run_app(app, vec![frame.to_vec()], None)
        .await
        .expect("app run failed");
    assert!(!out.is_empty());
    assert_eq!(setup.load(Ordering::SeqCst), 1);
    assert_eq!(teardown.load(Ordering::SeqCst), 1);
}
