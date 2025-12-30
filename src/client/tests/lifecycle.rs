//! Lifecycle hook tests for the wireframe client.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::helpers::{
    counting_hook,
    spawn_listener,
    test_error_hook_on_disconnect,
    test_with_client,
};
use crate::client::WireframeClient;

#[tokio::test]
async fn setup_callback_invoked_on_connect() {
    let (setup_count, increment) = counting_hook();

    let _client =
        test_with_client(|builder| builder.on_connection_setup(move || increment(42u32))).await;

    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should be invoked exactly once on connect"
    );
}

#[tokio::test]
async fn teardown_callback_receives_setup_state() {
    let teardown_value = Arc::new(AtomicUsize::new(0));
    let value = teardown_value.clone();

    let client = test_with_client(|builder| {
        builder
            .on_connection_setup(|| async { 42usize })
            .on_connection_teardown(move |state| {
                let value = value.clone();
                async move {
                    value.store(state, Ordering::SeqCst);
                }
            })
    })
    .await;

    client.close().await;

    assert_eq!(
        teardown_value.load(Ordering::SeqCst),
        42,
        "teardown callback should receive state from setup"
    );
}

#[tokio::test]
async fn teardown_without_setup_does_not_run() {
    let (teardown_count, increment) = counting_hook();

    let client = test_with_client(|builder| {
        builder.on_connection_teardown(move |value: ()| increment(value))
    })
    .await;

    client.close().await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        0,
        "teardown should not run when no setup hook was configured"
    );
}

#[tokio::test]
async fn setup_and_teardown_callbacks_run() {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let setup = setup_count.clone();
    let teardown = teardown_count.clone();

    let (addr, accept) = spawn_listener().await;

    let client = WireframeClient::builder()
        .on_connection_setup(move || {
            let setup = setup.clone();
            async move {
                setup.fetch_add(1, Ordering::SeqCst);
                "state"
            }
        })
        .on_connection_teardown(move |_: &str| {
            let teardown = teardown.clone();
            async move {
                teardown.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    let _server = accept.await.expect("join accept task");

    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should run exactly once"
    );

    client.close().await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        1,
        "teardown callback should run exactly once"
    );
}

#[tokio::test]
async fn on_connection_setup_preserves_error_hook() {
    // Configure on_error first, then on_connection_setup.
    // The error hook should be preserved.
    let error_count = test_error_hook_on_disconnect(|builder, count| {
        builder
            .on_error(move |_err| {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            })
            .on_connection_setup(|| async { 42u32 })
    })
    .await;

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error hook configured before on_connection_setup should be preserved"
    );
}

#[tokio::test]
async fn close_without_setup_does_not_invoke_teardown() {
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let count = teardown_count.clone();

    let (addr, accept) = spawn_listener().await;

    // Configure only teardown without setup - teardown should not run
    let client = WireframeClient::builder()
        .on_connection_teardown(move |(): ()| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    let _server = accept.await.expect("join accept task");
    client.close().await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        0,
        "teardown should not run when no setup hook produced state"
    );
}
