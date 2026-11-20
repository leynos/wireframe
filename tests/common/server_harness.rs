//! Transport harness shared by the server-facing test binaries.
//!
//! These helpers own the server lifecycle and the length-delimited client read,
//! with no knowledge of any particular protocol double. `server_push_support`
//! includes this module and re-exports its items for the push tests, while a
//! binary that installs a different protocol can include this module directly:
//! `#[path = "common/server_harness.rs"] mod server_harness;`.

use std::{net::SocketAddr, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::oneshot,
    time::timeout,
};
use wireframe::{
    app::WireframeApp,
    byte_order::read_network_u32,
    frame::LengthFormat,
    server::{ServerShutdown, WireframeServer},
};
use wireframe_testing::{TestResult, unused_listener, wait_for_server_readiness};

/// How long a test waits for a connection-scoped event before failing.
pub const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Read one length-delimited frame payload from `stream`.
///
/// Generic over the reader so the same framing serves a TCP peer and the
/// in-memory duplex transport a test substitutes for it.
pub async fn read_frame<R>(stream: &mut R) -> TestResult<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
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

/// Stop the server and wait for its accept loops to drain.
///
/// The wait is bounded so a connection task that fails to finish turns into a
/// test failure instead of a hung suite.
pub async fn stop(shutdown: &ServerShutdown) -> TestResult<()> {
    shutdown.stop();
    timeout(EVENT_TIMEOUT, shutdown.drained())
        .await
        .map_err(|_| "the server did not finish draining after a stop request")??;
    Ok(())
}
