//! Integration tests for the in-process server and client pair harness.
//!
//! These tests prove that [`spawn_wireframe_pair`] correctly orchestrates a
//! loopback server/client lifecycle and that the resulting [`WireframePair`]
//! exposes a usable client for request/response assertions.
#![cfg(not(loom))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rstest::rstest;
use wireframe::message::Message;
use wireframe_testing::{
    CommonTestEnvelope,
    Echo,
    TestResult,
    echo_app_factory,
    spawn_wireframe_pair,
    spawn_wireframe_pair_default,
};

// ── Default round trip ────────────────────────────────────────────────

#[rstest]
#[tokio::test]
async fn round_trip_via_pair_harness() -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let factory = echo_app_factory(&counter);

    let mut pair = spawn_wireframe_pair_default(factory).await?;

    let payload_bytes = Echo(42).to_bytes()?;
    let request = CommonTestEnvelope::new(1, Some(7), payload_bytes);
    let response: CommonTestEnvelope = pair.client_mut()?.call(&request).await?;

    if response.correlation_id != Some(7) {
        return Err("expected correlation id 7 on response".into());
    }
    let (echo, _) = Echo::from_bytes(&response.payload)?;
    if echo != Echo(42) {
        return Err("expected echo payload to match request".into());
    }
    if counter.load(Ordering::SeqCst) != 1 {
        return Err("expected handler to be invoked exactly once".into());
    }

    pair.shutdown().await
}

// ── Client-builder customization ──────────────────────────────────────

#[rstest]
#[tokio::test]
async fn custom_max_frame_length_via_pair_harness() -> TestResult<()> {
    use wireframe_testing::{CommonTestApp, echo_handler};

    let counter = Arc::new(AtomicUsize::new(0));

    // Create a factory that builds an app with a larger buffer capacity to
    // handle frames exceeding the default 4096-byte limit.
    let handler = echo_handler(&counter);
    let factory = move || -> TestResult<CommonTestApp> {
        let app = CommonTestApp::new()?.buffer_capacity(8192);
        Ok(app.route(1, handler.clone())?)
    };

    // Use a payload large enough to exceed the default 4096-byte frame limit.
    // Create a 5000-byte payload that will push the total frame size over the
    // default limit, requiring the custom max_frame_length(8192).
    let mut pair = spawn_wireframe_pair(factory, |builder| builder.max_frame_length(8192)).await?;

    let large_payload = vec![42u8; 5000];
    let request = CommonTestEnvelope::new(1, Some(42), large_payload.clone());
    let response: CommonTestEnvelope = pair.client_mut()?.call(&request).await?;

    if response.correlation_id != Some(42) {
        return Err("expected correlation id 42 on response".into());
    }
    if response.payload != large_payload {
        return Err("expected echoed payload to match request".into());
    }

    pair.shutdown().await
}

// ── Explicit shutdown ────────────────────────────────────────────────

#[rstest]
#[tokio::test]
async fn explicit_shutdown_completes_cleanly() -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let factory = echo_app_factory(&counter);

    let mut pair = spawn_wireframe_pair_default(factory).await?;

    // Verify the address is a valid loopback port.
    let addr = pair.local_addr();
    if !addr.ip().is_loopback() {
        return Err(format!("expected loopback address, got {addr}").into());
    }
    if addr.port() == 0 {
        return Err("expected a non-zero bound port".into());
    }

    pair.shutdown().await?;

    // Verify that client_mut returns an error after shutdown.
    if pair.client_mut().is_ok() {
        return Err("expected client_mut to return Err after shutdown".into());
    }

    Ok(())
}

// ── Drop safety net ──────────────────────────────────────────────────

#[rstest]
#[tokio::test]
async fn drop_without_explicit_shutdown_does_not_hang() -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let factory = echo_app_factory(&counter);

    let pair = spawn_wireframe_pair_default(factory).await?;

    // Drop without calling shutdown — the Drop impl should clean up
    // without blocking the test.
    drop(pair);

    // Yield so the runtime can process the shutdown signal.
    tokio::task::yield_now().await;
    Ok(())
}
