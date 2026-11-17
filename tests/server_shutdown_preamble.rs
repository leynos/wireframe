//! A server stop must release a connection that is still in its handshake.
//!
//! The preamble read sits in front of the application, and a peer that sends
//! only part of its preamble keeps that read pending. When the preamble
//! timeout is disabled, nothing else bounds it, so a shutdown token that the
//! handshake does not observe would leave the socket, the task and its
//! active-connection guard alive, and `ServerShutdown::drained` would wait for
//! the peer forever.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use tokio::{io::AsyncWriteExt, net::TcpStream, sync::oneshot, time::timeout};
use wireframe::{
    app::{Envelope, WireframeApp},
    connection::active_connection_count,
    serializer::BincodeSerializer,
    server::WireframeServer,
};
use wireframe_testing::{TestError, TestResult, unused_listener, wait_for_server_readiness};

/// Application type served after the handshake.
type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Preamble the client must send before the application is reached.
#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
struct Handshake {
    /// Magic bytes identifying the client.
    magic: [u8; 8],
    /// Protocol version the client speaks.
    version: u16,
}

/// Upper bound for observing the accepted connection.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound for the drain once the stop is requested.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Wait until the server has accepted a connection.
async fn await_accepted_connection() -> TestResult {
    let deadline = Instant::now() + ACCEPT_TIMEOUT;
    while active_connection_count() == 0 {
        if Instant::now() >= deadline {
            return Err(TestError::Msg(
                "the server never accepted the connection".into(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

/// A stop during an incomplete handshake must still drain the connection.
#[tokio::test]
async fn stop_releases_a_connection_stuck_in_its_preamble() -> TestResult {
    let server = WireframeServer::from_app(TestApp::default())
        .workers(1)
        .with_preamble::<Handshake>()
        // Without a timeout the read is bounded only by the peer or by a stop,
        // which is the case the shutdown token must cover.
        .disable_preamble_timeout();
    let listener = unused_listener()?;
    let bound = server.bind_existing_listener(listener)?;
    let addr = bound.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let shutdown = bound.ready_signal(ready_tx).spawn().await?;
    wait_for_server_readiness(ready_rx).await?;

    let mut client = TcpStream::connect(addr).await?;
    // Half a preamble leaves the decoder waiting for the rest of the struct.
    client
        .write_all(
            &Handshake {
                magic: *b"PARTIAL!",
                version: 1,
            }
            .magic[..4],
        )
        .await?;
    client.flush().await?;
    await_accepted_connection().await?;

    // The peer stays connected and silent, so only the stop can end the read.
    shutdown.stop();

    timeout(DRAIN_TIMEOUT, shutdown.drained())
        .await
        .map_err(|_| {
            TestError::Msg("the drain waited for a peer that never finished its preamble".into())
        })??;
    Ok(())
}
