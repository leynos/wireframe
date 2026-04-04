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
    let counter = Arc::new(AtomicUsize::new(0));
    let factory = echo_app_factory(&counter);

    let mut pair = spawn_wireframe_pair(factory, |builder| builder.max_frame_length(2048)).await?;

    let payload_bytes = Echo(99).to_bytes()?;
    let request = CommonTestEnvelope::new(1, Some(42), payload_bytes);
    let response: CommonTestEnvelope = pair.client_mut()?.call(&request).await?;

    if response.correlation_id != Some(42) {
        return Err("expected correlation id 42 on response".into());
    }
    let (echo, _) = Echo::from_bytes(&response.payload)?;
    if echo != Echo(99) {
        return Err("expected echo payload 99".into());
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

    pair.shutdown().await
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
