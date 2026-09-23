//! Lifecycle hook tests for the wireframe client.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures::future::lazy;

use super::helpers::{counting_hook, test_error_hook_on_disconnect, test_with_client};

fn record_count(count: Arc<AtomicUsize>) -> impl Future<Output = ()> + Send {
    lazy(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    })
}

async fn record_setup(count: Arc<AtomicUsize>) -> &'static str {
    record_count(count).await;
    "state"
}

fn record_teardown_state(value: Arc<AtomicUsize>, state: usize) -> impl Future<Output = ()> + Send {
    lazy(move |_| {
        value.store(state, Ordering::SeqCst);
    })
}

#[tokio::test]
async fn setup_callback_invoked_on_connect() {
    let (setup_count, increment) = counting_hook();

    let _client = test_with_client(|builder| builder.on_connection_setup(move || increment(42u32)))
        .await
        .expect("connect client");

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
            .on_connection_teardown(move |state| record_teardown_state(value.clone(), state))
    })
    .await
    .expect("connect client");

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
    .await
    .expect("connect client");

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

    let client = test_with_client(|builder| {
        builder
            .on_connection_setup(move || record_setup(setup.clone()))
            .on_connection_teardown(move |_: &str| record_count(teardown.clone()))
    })
    .await
    .expect("connect client");

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
            .on_error(move |_err| record_count(count.clone()))
            .on_connection_setup(|| async { 42u32 })
    })
    .await
    .expect("run error hook scenario");

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error hook configured before on_connection_setup should be preserved"
    );
}
