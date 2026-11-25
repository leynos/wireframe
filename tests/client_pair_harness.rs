//! Integration tests for the in-process server and client pair harness.
//!
//! These tests prove that [`spawn_wireframe_pair`] correctly orchestrates a
//! loopback server/client lifecycle and that the resulting [`WireframePair`]
//! exposes a usable client for request/response assertions.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

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

// ── Delegated lifecycle ──────────────────────────────────────────────

// The pair delegates to the server harness; these traits are what callers
// move across tasks, so pin them for the harness types as well as the pair.
static_assertions::assert_impl_all!(wireframe_testing::WireframePair: Send, Sync, Unpin);
static_assertions::assert_impl_all!(wireframe_testing::RunningWireframeServer: Send, Sync, Unpin);

/// Handle a request by waiting on `release` before answering.
fn gated_handler(
    started: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
) -> wireframe::app::ResponseHandler<wireframe::app::Envelope> {
    std::sync::Arc::new(move |_request: &wireframe::app::Envelope| {
        let handler_started = std::sync::Arc::clone(&started);
        let handler_release = std::sync::Arc::clone(&release);
        Box::pin(async move {
            handler_started.notify_one();
            handler_release.notified().await;
            wireframe::response::Response::Single(wireframe::app::Envelope::new(
                1,
                Some(7),
                Vec::new(),
            ))
        })
    })
}

/// Dropping the pair with a request in flight releases the listener promptly.
///
/// The pair historically aborted its server task after a 100 ms grace period.
/// With the handle delegation there is no task handle, so the guarantee comes
/// from the control's synchronous cancellation plus the server releasing its
/// listener when the accept loops exit. The parked handler proves the request
/// is still in flight when the release is observed, so neither the drop nor
/// the harness can have waited for it.
#[rstest]
#[tokio::test]
async fn drop_with_request_in_flight_releases_listener() -> TestResult {
    use std::{pin::pin, sync::Arc, time::Duration};

    use futures::poll;
    use tokio::{sync::Notify, time::timeout};
    use wireframe_testing::wait_for_listener_release;

    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let handler = gated_handler(Arc::clone(&started), Arc::clone(&release));
    let factory = move || -> TestResult<wireframe_testing::TestApp> {
        let app = wireframe_testing::TestApp::new()?;
        Ok(app.route_response(1, handler.clone())?)
    };

    let mut pair = spawn_wireframe_pair_default(factory).await?;
    let addr = pair.local_addr();
    let request = CommonTestEnvelope::new(1, Some(7), vec![1]);
    {
        let client = pair.client_mut()?;
        let call = client.call::<CommonTestEnvelope, CommonTestEnvelope>(&request);
        let mut pinned = pin!(call);
        if poll!(pinned.as_mut()).is_ready() {
            return Err("the gated request completed before the pair was dropped".into());
        }
    }
    timeout(Duration::from_secs(1), started.notified())
        .await
        .map_err(|_| "the gated handler never started".to_owned())?;

    drop(pair);
    wait_for_listener_release(addr).await?;

    // Release only after the release was observed, so the handler cannot have
    // finished before the listener closed.
    release.notify_one();
    Ok(())
}
