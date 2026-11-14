//! End-to-end coverage for push delivery while a server connection ends.
//!
//! A peer close is permanent and a server stop releases the connection, so
//! these tests describe both ways a connection stops serving pushes: frames
//! queued before a clean close are still delivered, and a push attempted after
//! a connection has ended is rejected rather than silently accepted.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

// The harness is shared with `server_push`, which uses the preload and
// setup-record helpers this binary does not drive.
#[expect(
    dead_code,
    reason = "the shared push harness also serves `server_push`"
)]
#[path = "common/server_push_support.rs"]
mod support;

use std::sync::Arc;

use bytes::Bytes;
use support::{
    EVENT_TIMEOUT,
    PushTestProtocol,
    only_connection,
    read_frame,
    spawn_server,
    stop,
    unblock_preamble,
    wait_until_registry_empty,
};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::Notify, time::timeout};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    byte_order::write_network_u32,
    push::PushError,
    serializer::{BincodeSerializer, Serializer},
    session::SessionRegistry,
};
use wireframe_testing::TestResult;

/// Server shutdown ends an idle connection and rejects further pushes.
///
/// The client stays connected and sends nothing, so neither the reader nor a
/// push lane can end the connection: only the connection loop observing the
/// server stop token can. The bounded drain in [`stop`] fails the test if that
/// observation is missing.
#[tokio::test]
async fn shutdown_ends_idle_connections() -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_session_registry(Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;
    let (connection_id, handle) = only_connection(&protocol).await?;

    stop(&shutdown).await?;
    wait_until_registry_empty(&registry).await?;
    if registry.get(&connection_id).is_some() {
        return Err("shutdown should deregister the idle connection".into());
    }

    let pushed = timeout(
        EVENT_TIMEOUT,
        handle.push_high_priority(Bytes::from_static(b"late")),
    )
    .await
    .map_err(|_| "a push after shutdown should not hang")?;
    match pushed {
        Err(PushError::Closed) => {}
        other => return Err(format!("shutdown should close the push lanes: {other:?}").into()),
    }

    drop(client);
    Ok(())
}

#[tokio::test]
async fn pushes_are_rejected_after_the_connection_closes() -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_session_registry(Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;
    let (_, handle) = only_connection(&protocol).await?;

    client.shutdown().await?;
    drop(client);
    wait_until_registry_empty(&registry).await?;

    let pushed = timeout(
        EVENT_TIMEOUT,
        handle.push_high_priority(Bytes::from_static(b"late")),
    )
    .await
    .map_err(|_| "a push to a closed connection should not hang")?;
    match pushed {
        Err(PushError::Closed) => {}
        other => {
            return Err(format!("a closed connection should reject pushes: {other:?}").into());
        }
    }

    stop(&shutdown).await
}

/// Write one length-delimited request envelope after the preamble is unblocked.
///
/// [`unblock_preamble`] has already written the leading zero byte, which the
/// default `()` preamble leaves buffered as the first byte of the application's
/// four-byte length prefix. This helper writes the rest of that prefix and then
/// the encoded envelope, so the route receives a well-formed frame.
async fn write_request(stream: &mut TcpStream, route_id: u32) -> TestResult<()> {
    let envelope = Envelope::new(route_id, Some(1), Vec::new());
    let payload = BincodeSerializer.serialize(&envelope)?;
    let prefix = write_network_u32(u32::try_from(payload.len())?);
    let (leading, remainder) = prefix
        .split_first()
        .ok_or("a four-byte length prefix is never empty")?;
    if *leading != 0 {
        return Err("request payload exceeds the preamble-primed length prefix".into());
    }
    stream.write_all(remainder).await?;
    stream.write_all(&payload).await?;
    Ok(())
}

/// Frames still queued when the peer half-closes are delivered before exit.
///
/// The route handler holds the connection loop inside dispatch, so both pushes
/// are still sitting in their lanes when the client's write half closes. A
/// clean end-of-stream must drain those lanes, high priority before low, rather
/// than discard frames a producer was told had been accepted.
#[tokio::test]
async fn queued_pushes_are_drained_after_a_clean_peer_close() -> TestResult<()> {
    let protocol = PushTestProtocol::new();
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let handler_started = Arc::clone(&started);
    let handler_gate = Arc::clone(&gate);
    let handler: Handler<Envelope> = Arc::new(move |_request: &Envelope| {
        let route_started = Arc::clone(&handler_started);
        let route_gate = Arc::clone(&handler_gate);
        Box::pin(async move {
            route_started.notify_one();
            route_gate.notified().await;
        })
    });
    let app = WireframeApp::default()
        .route(1, handler)?
        .with_protocol(protocol.clone());
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;
    let (_, handle) = only_connection(&protocol).await?;

    write_request(&mut client, 1).await?;
    timeout(EVENT_TIMEOUT, started.notified())
        .await
        .map_err(|_| "the route handler did not start")?;

    handle
        .push_high_priority(Bytes::from_static(b"drained-high"))
        .await
        .map_err(|error| format!("high-priority push should be accepted: {error:?}"))?;
    handle
        .push_low_priority(Bytes::from_static(b"drained-low"))
        .await
        .map_err(|error| format!("low-priority push should be accepted: {error:?}"))?;

    client.shutdown().await?;
    gate.notify_one();

    // The handler's own response is written first; the drained pushes follow.
    timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    let high = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    let low = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if high != b"drained-high" || low != b"drained-low" {
        return Err(format!("drain should deliver high then low: {high:?}, {low:?}").into());
    }

    drop(client);
    stop(&shutdown).await
}
