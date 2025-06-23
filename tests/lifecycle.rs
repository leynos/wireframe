use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::BytesMut;
use tokio::io::duplex;
use wireframe::{
    app::WireframeApp,
    frame::{FrameProcessor, LengthPrefixedProcessor},
    serializer::{BincodeSerializer, Serializer},
};

mod util;
use util::{processor, run_app_with_frame};

#[tokio::test]
async fn setup_and_teardown_callbacks_run() {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));

    let setup_clone = setup_count.clone();
    let teardown_clone = teardown_count.clone();

    let app = WireframeApp::new()
        .unwrap()
        .on_connection_setup(move || {
            let setup_clone = setup_clone.clone();
            async move {
                setup_clone.fetch_add(1, Ordering::SeqCst);
                42u32
            }
        })
        .unwrap()
        .on_connection_teardown(move |state| {
            let teardown_clone = teardown_clone.clone();
            async move {
                assert_eq!(state, 42u32);
                teardown_clone.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();

    let (_client, server) = duplex(64);
    app.handle_connection(server).await;

    assert_eq!(setup_count.load(Ordering::SeqCst), 1);
    assert_eq!(teardown_count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn setup_without_teardown_runs() {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let setup_clone = setup_count.clone();

    let app = WireframeApp::new()
        .unwrap()
        .on_connection_setup(move || {
            let setup_clone = setup_clone.clone();
            async move {
                setup_clone.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();

    let (_client, server) = duplex(64);
    app.handle_connection(server).await;

    assert_eq!(setup_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn teardown_without_setup_does_not_run() {
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let teardown_clone = teardown_count.clone();

    let app = WireframeApp::new()
        .unwrap()
        .on_connection_teardown(move |()| {
            let teardown_clone = teardown_clone.clone();
            async move {
                teardown_clone.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();

    let (_client, server) = duplex(64);
    app.handle_connection(server).await;

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

    let setup_clone = setup.clone();
    let teardown_clone = teardown.clone();

    let app = WireframeApp::<_, _, StateEnvelope>::new_with_envelope()
        .unwrap()
        .frame_processor(processor())
        .on_connection_setup(move || {
            let setup_clone = setup_clone.clone();
            async move {
                setup_clone.fetch_add(1, Ordering::SeqCst);
                7u32
            }
        })
        .unwrap()
        .on_connection_teardown(move |state| {
            let teardown_clone = teardown_clone.clone();
            async move {
                assert_eq!(state, 7u32);
                teardown_clone.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap()
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
