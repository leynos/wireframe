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
    app::{Envelope, Packet, WireframeApp},
    frame::{FrameProcessor, LengthPrefixedProcessor},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{processor, run_app_with_frame, run_with_duplex_server};

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

    WireframeApp::<_, _, E>::new_with_envelope()
        .unwrap()
        .on_connection_setup(move || setup_cb(()))
        .unwrap()
        .on_connection_teardown(teardown_cb)
        .unwrap()
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

    let app = WireframeApp::new()
        .unwrap()
        .on_connection_setup(move || cb(()))
        .unwrap();

    run_with_duplex_server(app).await;

    assert_eq!(setup_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn teardown_without_setup_does_not_run() {
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let cb = call_counting_callback(&teardown_count, ());

    let app = WireframeApp::new()
        .unwrap()
        .on_connection_teardown(cb)
        .unwrap();

    run_with_duplex_server(app).await;

    assert_eq!(teardown_count.load(Ordering::SeqCst), 0);
}

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct StateEnvelope {
    id: u32,
    msg: Vec<u8>,
}

impl wireframe::app::Packet for StateEnvelope {
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> (u32, Vec<u8>) { (self.id, self.msg) }

    fn from_parts(id: u32, msg: Vec<u8>) -> Self { Self { id, msg } }
}

#[tokio::test]
async fn helpers_propagate_connection_state() {
    let setup = Arc::new(AtomicUsize::new(0));
    let teardown = Arc::new(AtomicUsize::new(0));

    let app = wireframe_app_with_lifecycle_callbacks::<StateEnvelope>(&setup, &teardown, 7)
        .frame_processor(processor())
        .route(1, Arc::new(|_: &StateEnvelope| Box::pin(async {})))
        .unwrap();

    let env = StateEnvelope {
        id: 1,
        msg: vec![1],
    };
    let bytes = BincodeSerializer.serialize(&env).unwrap();
    let mut frame = BytesMut::new();
    LengthPrefixedProcessor::default()
        .encode(&bytes, &mut frame)
        .unwrap();

    let out = run_app_with_frame(app, frame.to_vec()).await.unwrap();
    assert!(!out.is_empty());
    assert_eq!(setup.load(Ordering::SeqCst), 1);
    assert_eq!(teardown.load(Ordering::SeqCst), 1);
}
