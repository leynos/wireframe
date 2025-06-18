use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::io::duplex;
use wireframe::app::WireframeApp;

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
