//! Harness for the server push end-to-end tests.
//!
//! The protocol records the [`PushHandle`] it receives at setup so a test can
//! push frames the way application code would, while the server lifecycle and
//! length-delimited client reads come from `server_harness`, re-exported here
//! so a push test imports everything from one module. The constructor for
//! frames preloaded during setup lives in `server_push_preload`, so a binary
//! that never preloads a frame does not carry it.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use tokio::time::{Instant, sleep};
use wireframe::{
    hooks::{ConnectionContext, WireframeProtocol},
    push::{PushHandle, PushPolicy, PushPriority},
    session::{ConnectionId, SessionRegistry},
};
use wireframe_testing::TestResult;

#[path = "server_harness.rs"]
mod server_harness;

pub use server_harness::{EVENT_TIMEOUT, read_frame, spawn_server, stop};

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
    pub(crate) setups: Arc<Mutex<Vec<SetupRecord>>>,
    /// Frames queued during setup, before the connection reads anything.
    ///
    /// Set through the `server_push_preload` constructor, which only the
    /// binary that tests preloaded frames compiles.
    pub(crate) preloaded: Arc<Vec<(Bytes, PushPriority)>>,
}

impl PushTestProtocol {
    /// Build a protocol with no preloaded frames.
    pub fn new() -> Self { Self::default() }

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

/// Return the identifier and handle of the single connection that completed setup.
pub async fn only_connection(
    protocol: &PushTestProtocol,
) -> TestResult<(ConnectionId, PushHandle<Bytes>)> {
    let records = protocol.wait_for_setups(1).await?;
    let record = records.first().ok_or("expected a setup record")?;
    let connection_id = record.connection_id.ok_or("setup should see an id")?;
    Ok((connection_id, record.handle.clone()))
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
