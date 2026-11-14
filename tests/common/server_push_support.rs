//! Harness for the server push end-to-end tests.
//!
//! The protocol records the [`PushHandle`] it receives at setup so a test can
//! push frames the way application code would, while the helpers own the
//! server lifecycle and the length-delimited client reads.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
    time::{Instant, sleep},
};
use wireframe::{
    app::WireframeApp,
    byte_order::read_network_u32,
    frame::LengthFormat,
    hooks::{ConnectionContext, WireframeProtocol},
    push::{PushHandle, PushPolicy, PushPriority},
    server::{ServerShutdown, WireframeServer},
    session::{ConnectionId, SessionRegistry},
};
use wireframe_testing::{TestResult, unused_listener, wait_for_server_readiness};

/// How long a test waits for a connection-scoped event before failing.
pub const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// A push handle and the connection identifier announced at setup.
pub struct SetupRecord {
    /// Identifier the server allocated for the connection.
    pub connection_id: Option<ConnectionId>,
    /// Handle application code would retain for asynchronous pushes.
    pub handle: PushHandle<Bytes>,
}

/// Protocol that records setup state and optionally queues frames immediately.
#[derive(Clone, Default)]
pub struct PushTestProtocol {
    /// Records captured by `on_connection_setup`, oldest first.
    setups: Arc<Mutex<Vec<SetupRecord>>>,
    /// Frames queued during setup, before the connection reads anything.
    preloaded: Arc<Vec<(Bytes, PushPriority)>>,
}

impl PushTestProtocol {
    /// Build a protocol with no preloaded frames.
    pub fn new() -> Self { Self::default() }

    /// Build a protocol that queues `preloaded` during every setup.
    pub fn with_preloaded(preloaded: Vec<(Bytes, PushPriority)>) -> Self {
        Self {
            preloaded: Arc::new(preloaded),
            ..Self::default()
        }
    }

    /// Wait until at least `count` connections have completed setup.
    pub async fn wait_for_setups(&self, count: usize) -> TestResult<Vec<SetupRecord>> {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            if let Some(records) = self.snapshot(count) {
                return Ok(records);
            }
            if Instant::now() >= deadline {
                return Err("connections did not complete setup in time".into());
            }
            sleep(Duration::from_millis(5)).await;
        }
    }

    /// Copy the setup records once at least `count` are present.
    fn snapshot(&self, count: usize) -> Option<Vec<SetupRecord>> {
        let records = self.setups.lock().expect("setup records poisoned");
        if records.len() < count {
            return None;
        }
        Some(
            records
                .iter()
                .map(|record| SetupRecord {
                    connection_id: record.connection_id,
                    handle: record.handle.clone(),
                })
                .collect(),
        )
    }
}

impl WireframeProtocol for PushTestProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn on_connection_setup(&self, handle: PushHandle<Self::Frame>, ctx: &mut ConnectionContext) {
        for (frame, priority) in self.preloaded.iter() {
            handle
                .try_push(frame.clone(), *priority, PushPolicy::DropIfFull)
                .expect("preloaded push should be accepted or dead-lettered");
        }
        self.setups
            .lock()
            .expect("setup records poisoned")
            .push(SetupRecord {
                connection_id: ctx.connection_id(),
                handle,
            });
    }
}

/// Start a server for `app` and wait until it is accepting connections.
pub async fn spawn_server(app: WireframeApp) -> TestResult<(SocketAddr, ServerShutdown)> {
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let shutdown = server.ready_signal(ready_tx).spawn().await?;
    wait_for_server_readiness(ready_rx).await?;
    Ok((addr, shutdown))
}

/// Send the byte the preamble reader insists on before it hands the stream on.
///
/// The default preamble decodes no value but the reader still fills a one-byte
/// buffer first, leaving that byte rewound for the application. A client that
/// writes nothing would never let the connection reach `on_connection_setup`.
/// The stray byte stays buffered in the application's length-prefixed decoder
/// and is never completed into a frame.
pub async fn unblock_preamble(stream: &mut TcpStream) -> TestResult<()> {
    stream.write_all(b"\0").await?;
    Ok(())
}

/// Read one length-delimited frame payload from the client socket.
pub async fn read_frame(stream: &mut TcpStream) -> TestResult<Vec<u8>> {
    let mut prefix = vec![0u8; LengthFormat::default().bytes()];
    stream.read_exact(&mut prefix).await?;
    let length = read_network_u32(
        prefix
            .as_slice()
            .try_into()
            .map_err(|_| "frame prefix should be four bytes")?,
    );
    let mut payload = vec![0u8; usize::try_from(length)?];
    stream.read_exact(&mut payload).await?;
    Ok(payload)
}

/// Return the identifier and handle of the single connection that completed setup.
pub async fn only_connection(
    protocol: &PushTestProtocol,
) -> TestResult<(ConnectionId, PushHandle<Bytes>)> {
    let records = protocol.wait_for_setups(1).await?;
    let record = records.first().ok_or("expected a setup record")?;
    let connection_id = record.connection_id.ok_or("setup should see an id")?;
    Ok((connection_id, record.handle.clone()))
}

/// Stop the server and wait for its accept loops to drain.
pub async fn stop(shutdown: &ServerShutdown) -> TestResult<()> {
    shutdown.stop();
    shutdown.drained().await?;
    Ok(())
}

/// Poll the registry until it reports no live connections.
pub async fn wait_until_registry_empty(registry: &SessionRegistry<Bytes>) -> TestResult<()> {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    while !registry.active_ids().is_empty() {
        if Instant::now() >= deadline {
            return Err("closed connections were not deregistered".into());
        }
        sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}
