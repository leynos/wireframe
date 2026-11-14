//! End-to-end coverage for server pushes delivered through [`WireframeServer`].
//!
//! Every test drives the public server surface over a real TCP socket. The
//! protocol records the [`PushHandle`] it receives at connection setup, so the
//! assertions describe what an application can observe: frames written to the
//! client, the registry entry's lifetime, and the rejection of pushes once the
//! connection has ended.
#![cfg(not(loom))]

#[path = "common/server_push_support.rs"]
mod support;

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use support::{
    EVENT_TIMEOUT,
    PushTestProtocol,
    expect_no_frame,
    only_connection,
    read_frame,
    spawn_server,
    stop,
    unblock_preamble,
    wait_until_registry_empty,
};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc, time::timeout};
use wireframe::{
    app::WireframeApp,
    push::{PushError, PushQueues},
    session::{ConnectionId, SessionRegistry},
};
use wireframe_testing::TestResult;

#[tokio::test]
async fn setup_handle_delivers_push_over_tcp() -> TestResult<()> {
    let protocol = PushTestProtocol::new();
    let (addr, shutdown) =
        spawn_server(WireframeApp::default().with_protocol(protocol.clone())).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;

    let (_, handle) = only_connection(&protocol).await?;
    handle
        .push_high_priority(Bytes::from_static(b"hello"))
        .await
        .map_err(|error| format!("push should be accepted: {error:?}"))?;

    let payload = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if payload != b"hello" {
        return Err(format!("unexpected pushed payload: {payload:?}").into());
    }

    drop(client);
    stop(&shutdown).await
}

#[tokio::test]
async fn high_priority_pushes_precede_low_priority_pushes() -> TestResult<()> {
    use wireframe::push::PushPriority;

    let protocol = PushTestProtocol::with_preloaded(vec![
        (Bytes::from_static(b"urgent"), PushPriority::High),
        (Bytes::from_static(b"best-effort"), PushPriority::Low),
    ]);
    let (addr, shutdown) =
        spawn_server(WireframeApp::default().with_protocol(protocol.clone())).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;
    protocol.wait_for_setups(1).await?;

    let first = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    let second = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if first != b"urgent" || second != b"best-effort" {
        return Err(format!("pushes arrived out of priority order: {first:?}, {second:?}").into());
    }

    drop(client);
    stop(&shutdown).await
}

#[tokio::test]
async fn connection_setup_registers_the_push_handle() -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_session_registry(Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;

    let (connection_id, _) = only_connection(&protocol).await?;
    if registry.get(&connection_id).is_none() {
        return Err("registry should hold the connection's push handle".into());
    }

    drop(client);
    stop(&shutdown).await
}

#[tokio::test]
async fn registry_entry_is_removed_when_connection_closes() -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_session_registry(Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;
    let (connection_id, _) = only_connection(&protocol).await?;

    client.shutdown().await?;
    drop(client);
    wait_until_registry_empty(&registry).await?;
    if registry.get(&connection_id).is_some() {
        return Err("closed connection should be deregistered".into());
    }

    stop(&shutdown).await
}

#[tokio::test]
async fn concurrent_connections_receive_distinct_ids() -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_session_registry(Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let mut first = TcpStream::connect(addr).await?;
    let mut second = TcpStream::connect(addr).await?;
    unblock_preamble(&mut first).await?;
    unblock_preamble(&mut second).await?;

    let records = protocol.wait_for_setups(2).await?;
    let mut ids: Vec<ConnectionId> = records
        .iter()
        .map(|record| record.connection_id.ok_or("setup should see an id"))
        .collect::<Result<_, _>>()?;
    ids.sort_by_key(ConnectionId::as_u64);
    ids.dedup();
    if ids.len() != 2 {
        return Err(format!("connections should have distinct ids: {ids:?}").into());
    }
    if registry.active_ids().len() != 2 {
        return Err("both live connections should be registered".into());
    }

    drop((first, second));
    stop(&shutdown).await
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

#[tokio::test]
async fn dead_letter_queue_receives_frames_a_full_queue_drops() -> TestResult<()> {
    let (dlq_tx, mut dlq_rx) = mpsc::channel(4);
    let template = PushQueues::<Bytes>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .dlq(Some(dlq_tx));
    let protocol = PushTestProtocol::with_preloaded(vec![
        (
            Bytes::from_static(b"kept"),
            wireframe::push::PushPriority::High,
        ),
        (
            Bytes::from_static(b"dropped-a"),
            wireframe::push::PushPriority::High,
        ),
        (
            Bytes::from_static(b"dropped-b"),
            wireframe::push::PushPriority::High,
        ),
    ]);
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_push_queues(template);
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;
    protocol.wait_for_setups(1).await?;

    let payload = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if payload != b"kept" {
        return Err(format!("the queued frame should be delivered: {payload:?}").into());
    }
    for expected in [&b"dropped-a"[..], &b"dropped-b"[..]] {
        let dropped = timeout(EVENT_TIMEOUT, dlq_rx.recv())
            .await?
            .ok_or("dead-letter queue closed early")?;
        if dropped != expected {
            return Err(format!("unexpected dead-letter frame: {dropped:?}").into());
        }
    }

    drop(client);
    stop(&shutdown).await
}

#[tokio::test]
async fn rate_limited_push_is_delayed_but_delivered() -> TestResult<()> {
    let template = PushQueues::<Bytes>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(1));
    let protocol = PushTestProtocol::new();
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_push_queues(template);
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
    unblock_preamble(&mut client).await?;
    let (_, handle) = only_connection(&protocol).await?;

    handle
        .push_low_priority(Bytes::from_static(b"first"))
        .await
        .map_err(|error| format!("first push should be accepted: {error:?}"))?;
    let first = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if first != b"first" {
        return Err(format!("unexpected first payload: {first:?}").into());
    }

    // The single token for this window is spent, so the next low-priority push
    // must wait for the refill instead of queueing immediately.
    let pending = tokio::spawn(async move {
        handle
            .push_low_priority(Bytes::from_static(b"second"))
            .await
    });
    expect_no_frame(&mut client, Duration::from_millis(200)).await?;

    let second = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if second != b"second" {
        return Err(format!("unexpected second payload: {second:?}").into());
    }
    pending.await??;

    drop(client);
    stop(&shutdown).await
}
