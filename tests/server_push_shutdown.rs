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
    wait_until_registry_empty,
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpSocket, TcpStream},
    time::timeout,
};
use wireframe::{
    app::WireframeApp,
    codec::LengthDelimitedFrameCodec,
    push::PushError,
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
    let client = TcpStream::connect(addr).await?;
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

/// A stop request deregisters a connection whose push write cannot progress.
///
/// The client never reads and asks for a tiny receive buffer, so a frame far
/// larger than the socket buffers stays in flight. The stop token abandons
/// that write rather than waiting for the peer, and the connection's registry
/// guard must still remove the entry.
#[tokio::test]
async fn shutdown_deregisters_a_connection_with_a_pending_push() -> TestResult<()> {
    /// Payload larger than the buffers a loopback write could drain.
    const IN_FLIGHT: usize = 16 * 1024 * 1024 - 1;
    /// Maximum frame length accepted by the connection codec.
    const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;
    /// Client receive buffer, small enough to stall the write after one window.
    const RECEIVE_BUFFER: u32 = 4096;

    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    // The default app fixes the codec type the replacement is built from.
    let default_app: WireframeApp = WireframeApp::default();
    let app = default_app
        .with_codec(LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH))
        .with_protocol(protocol.clone())
        .with_session_registry(Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let socket = TcpSocket::new_v4()?;
    socket.set_recv_buffer_size(RECEIVE_BUFFER)?;
    let client = socket.connect(addr).await?;
    let (connection_id, handle) = only_connection(&protocol).await?;

    handle
        .push_high_priority(Bytes::from(vec![b'p'; IN_FLIGHT]))
        .await
        .map_err(|error| format!("the pending push should be accepted: {error:?}"))?;
    // Let the actor take the frame, so the stop lands while the write blocks
    // on a peer that is not draining its receive window.
    tokio::task::yield_now().await;

    stop(&shutdown).await?;
    wait_until_registry_empty(&registry).await?;
    if registry.get(&connection_id).is_some() {
        return Err("shutdown should deregister a connection with a pending push".into());
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

/// Frames still queued when the peer half-closes are drained before exit.
///
/// The transport is an in-memory duplex stream whose buffer is smaller than the
/// first frame, so that frame's write stays pending until the peer reads it. The
/// backlog is therefore established rather than presumed: the first frame is in
/// flight and the next two are still queued in their lanes when the peer closes
/// its write half. Reading the in-flight frame lets its write finish, so the
/// frames that follow can only have come from the drain, high priority before
/// low. The peer's read half stays open, so the connection ends on the
/// half-close rather than on socket cleanup.
#[tokio::test]
async fn queued_pushes_are_drained_after_a_clean_peer_close() -> TestResult<()> {
    /// Buffer size of the in-memory transport, in bytes.
    const BUFFER: usize = 64;
    /// Payload that cannot fit in the buffer, so its write has to block.
    const IN_FLIGHT: usize = 256;

    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app: WireframeApp = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_session_registry(Arc::clone(&registry));
    let prepared = app
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let prepared_template = Arc::new(prepared);
    let (mut peer, server) = tokio::io::duplex(BUFFER);
    // The connection borrows the prepared template, so the task owns a handle
    // to it rather than a borrow that cannot outlive this function.
    let connection = tokio::spawn({
        let owned_template = Arc::clone(&prepared_template);
        async move { owned_template.handle_connection_result(server).await }
    });
    let (_, handle) = only_connection(&protocol).await?;

    handle
        .push_high_priority(Bytes::from(vec![b'a'; IN_FLIGHT]))
        .await
        .map_err(|error| format!("in-flight push should be accepted: {error:?}"))?;
    // The high lane holds one frame, so this push cannot complete until the
    // loop has taken the first one. Taken together with a frame too large for
    // the transport buffer, that leaves the first write in flight here.
    handle
        .push_high_priority(Bytes::from_static(b"drained-high"))
        .await
        .map_err(|error| format!("queued high-priority push should be accepted: {error:?}"))?;
    handle
        .push_low_priority(Bytes::from_static(b"drained-low"))
        .await
        .map_err(|error| format!("queued low-priority push should be accepted: {error:?}"))?;

    peer.shutdown().await?;

    let in_flight = timeout(EVENT_TIMEOUT, read_frame(&mut peer)).await??;
    if in_flight != vec![b'a'; IN_FLIGHT] {
        return Err("the in-flight frame should arrive intact".into());
    }
    let high = timeout(EVENT_TIMEOUT, read_frame(&mut peer)).await??;
    let low = timeout(EVENT_TIMEOUT, read_frame(&mut peer)).await??;
    if high != b"drained-high" || low != b"drained-low" {
        return Err(format!("drain should deliver high then low: {high:?}, {low:?}").into());
    }

    // The read half stays open, so an emptied registry proves the connection
    // ended on the half-close rather than when the socket dropped.
    wait_until_registry_empty(&registry).await?;
    let joined = timeout(EVENT_TIMEOUT, connection)
        .await
        .map_err(|_| "the connection task should finish after the drain")??;
    joined?;

    let pushed = timeout(
        EVENT_TIMEOUT,
        handle.push_high_priority(Bytes::from_static(b"late")),
    )
    .await
    .map_err(|_| "a push after the drain should not hang")?;
    match pushed {
        Err(PushError::Closed) => Ok(()),
        other => Err(format!("a drained connection should reject pushes: {other:?}").into()),
    }
}
