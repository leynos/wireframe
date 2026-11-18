//! End-to-end coverage for server pushes delivered through [`WireframeServer`].
//!
//! Every test drives the public server surface over a real TCP socket. The
//! protocol records the [`PushHandle`] it receives at connection setup, so the
//! assertions describe what an application can observe: frames written to the
//! client, the registry entry's lifetime, and the rejection of pushes once the
//! connection has ended. Shutdown and peer-close behaviour live in
//! `server_push_shutdown.rs`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/server_push_support.rs"]
mod support;
// The preload constructor is an inherent impl, so this declaration adds it to
// this binary without naming the module at any call site.
#[path = "common/server_push_preload.rs"]
mod preload_support;

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
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::mpsc, time::timeout};
use wireframe::{
    app::WireframeApp,
    hooks::{ConnectionContext, WireframeProtocol},
    push::{PushHandle, PushPriority, PushQueues},
    session::{ConnectionId, SessionRegistry},
};
use wireframe_testing::TestResult;

/// Build an application whose connections publish into `registry`.
///
/// Takes the protocol by value because the caller keeps a clone to observe the
/// setups, while the server owns the application's copy.
fn registered_app<P>(protocol: P, registry: Arc<SessionRegistry<Bytes>>) -> WireframeApp
where
    P: WireframeProtocol<Frame = Bytes, ProtocolError = ()> + 'static,
{
    WireframeApp::default()
        .with_protocol(protocol)
        .with_session_registry(registry)
}

/// Wait for `connection_id` to leave `registry`, failing with `message`.
///
/// The wait is what makes the assertion meaningful: the entry is removed when
/// the connection task ends, so looking it up too early would race the task
/// rather than test the guard.
async fn assert_deregistered(
    registry: &SessionRegistry<Bytes>,
    connection_id: &ConnectionId,
    message: &str,
) -> TestResult<()> {
    wait_until_registry_empty(registry).await?;
    if registry.get(connection_id).is_some() {
        return Err(message.into());
    }
    Ok(())
}

/// A client that sends nothing still receives a push.
///
/// The default `()` preamble consumes no bytes, so the connection reaches
/// `on_connection_setup` and registers its push handle without the client
/// writing anything. A server that waited for the first inbound byte here
/// would leave a silent client waiting for an unsolicited frame.
#[tokio::test]
async fn silent_client_receives_a_push_after_setup() -> TestResult<()> {
    let protocol = PushTestProtocol::new();
    let (addr, shutdown) =
        spawn_server(WireframeApp::default().with_protocol(protocol.clone())).await?;
    let mut client = TcpStream::connect(addr).await?;

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

/// The low-priority frame is queued first, so only priority ordering can put
/// the urgent frame ahead of it; a FIFO drain would deliver them the other way
/// round.
#[tokio::test]
async fn high_priority_pushes_precede_low_priority_pushes() -> TestResult<()> {
    let protocol = PushTestProtocol::with_preloaded(vec![
        (Bytes::from_static(b"best-effort"), PushPriority::Low),
        (Bytes::from_static(b"urgent"), PushPriority::High),
    ]);
    let (addr, shutdown) =
        spawn_server(WireframeApp::default().with_protocol(protocol.clone())).await?;
    let mut client = TcpStream::connect(addr).await?;
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
    let app = registered_app(protocol.clone(), Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let client = TcpStream::connect(addr).await?;

    let (connection_id, _) = only_connection(&protocol).await?;
    if registry.get(&connection_id).is_none() {
        return Err("registry should hold the connection's push handle".into());
    }

    drop(client);
    stop(&shutdown).await
}

/// How a client ends its connection, which decides what the server observes.
///
/// A half-close sends end-of-file, which the server sees as a clean close. A
/// zero-length `SO_LINGER` instead makes `drop` send a reset, so the server's
/// read fails with `ECONNRESET`: an I/O error rather than a clean close.
#[derive(Clone, Copy, Debug)]
enum ClientTeardown {
    /// Half-close the connection, then drop it.
    HalfClose,
    /// Drop it with a zero-length linger so the peer reads a reset.
    Reset,
}

impl ClientTeardown {
    /// End `client` this way, so the server observes the matching close.
    async fn apply(self, mut client: TcpStream) -> TestResult<()> {
        match self {
            Self::HalfClose => client.shutdown().await?,
            Self::Reset => client.set_zero_linger()?,
        }
        drop(client);
        Ok(())
    }

    /// What a failure to deregister after this teardown should say.
    fn deregistration_message(self) -> &'static str {
        match self {
            Self::HalfClose => "closed connection should be deregistered",
            Self::Reset => "a reset connection should be deregistered",
        }
    }
}

/// However a peer ends the connection, its registry entry is removed.
///
/// The entry is published at setup and removed when the connection task ends,
/// so neither a clean close nor an I/O error may strand it.
#[rstest::rstest]
#[case::clean_close(ClientTeardown::HalfClose)]
#[case::reset(ClientTeardown::Reset)]
#[tokio::test]
async fn registry_entry_is_removed_when_the_peer_ends(
    #[case] teardown: ClientTeardown,
) -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = registered_app(protocol.clone(), Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let client = TcpStream::connect(addr).await?;
    let (connection_id, _) = only_connection(&protocol).await?;

    teardown.apply(client).await?;

    assert_deregistered(&registry, &connection_id, teardown.deregistration_message()).await?;
    stop(&shutdown).await
}

/// Protocol that panics in `on_connection_setup`, after recording the setup.
///
/// The protocol callback runs immediately after the connection publishes its
/// registry entry, so this is the earliest point a connection can fail while
/// it is registered. The recorded setup keeps a strong handle alive, so an
/// entry the guard failed to remove would still upgrade rather than look like
/// an already-dead weak reference.
#[derive(Clone)]
struct PanickingSetupProtocol(PushTestProtocol);

impl WireframeProtocol for PanickingSetupProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn on_connection_setup(&self, handle: PushHandle<Self::Frame>, ctx: &mut ConnectionContext) {
        WireframeProtocol::on_connection_setup(&self.0, handle, ctx);
        panic!("connection setup panic");
    }
}

/// A panic while a connection sets up still deregisters it.
///
/// The application's own `on_connection_setup` hook runs before the registry
/// entry is published, so a panic there would leave nothing to observe. This
/// panics in the protocol callback instead, which runs after publication, and
/// waits for the connection to be torn down before looking the entry up.
#[tokio::test]
async fn registry_entry_is_removed_when_connection_setup_panics() -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = registered_app(
        PanickingSetupProtocol(protocol.clone()),
        Arc::clone(&registry),
    );
    let (addr, shutdown) = spawn_server(app).await?;
    let _client = TcpStream::connect(addr).await?;

    // Recording the setup proves the entry was published before the panic.
    let (connection_id, _) = only_connection(&protocol).await?;

    assert_deregistered(
        &registry,
        &connection_id,
        "a connection whose setup panicked should be deregistered",
    )
    .await?;
    stop(&shutdown).await
}

#[tokio::test]
async fn concurrent_connections_receive_distinct_ids() -> TestResult<()> {
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let protocol = PushTestProtocol::new();
    let app = registered_app(protocol.clone(), Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let first = TcpStream::connect(addr).await?;
    let second = TcpStream::connect(addr).await?;

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
async fn dead_letter_queue_receives_frames_a_full_queue_drops() -> TestResult<()> {
    let (dlq_tx, mut dlq_rx) = mpsc::channel(4);
    let template = PushQueues::<Bytes>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .dlq(Some(dlq_tx));
    let protocol = PushTestProtocol::with_preloaded(vec![
        (Bytes::from_static(b"kept"), PushPriority::High),
        (Bytes::from_static(b"dropped-a"), PushPriority::High),
        (Bytes::from_static(b"dropped-b"), PushPriority::High),
    ]);
    let app = WireframeApp::default()
        .with_protocol(protocol.clone())
        .with_push_queues(template);
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;
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

/// A rate-limited queue still delivers every push over TCP.
///
/// The limiter allows one push per second, so this exercises the throttled
/// delivery path end to end. The delay itself is asserted against a paused
/// clock in `tests/push.rs::rate_limiter_blocks_when_exceeded`; timing it here
/// would measure socket setup against the wall clock rather than the limiter.
#[tokio::test]
async fn rate_limited_pushes_are_delivered_over_tcp() -> TestResult<()> {
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
    let (_, handle) = only_connection(&protocol).await?;

    handle
        .push_low_priority(Bytes::from_static(b"first"))
        .await
        .map_err(|error| format!("first push should be accepted: {error:?}"))?;
    let first = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if first != b"first" {
        return Err(format!("unexpected first payload: {first:?}").into());
    }

    // Bounded, so a limiter that never replenishes fails the test instead of
    // leaving the second push pending past the frame read below.
    timeout(
        EVENT_TIMEOUT,
        handle.push_low_priority(Bytes::from_static(b"second")),
    )
    .await
    .map_err(|_| "the rate limiter should replenish a token")?
    .map_err(|error| format!("second push should be accepted: {error:?}"))?;

    let second = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    if second != b"second" {
        return Err(format!("unexpected second payload: {second:?}").into());
    }

    drop(client);
    stop(&shutdown).await
}
