//! Test world state for Cucumber panic resilience tests.
//!
//! Provides shared state management for behavioural tests verifying
//! server resilience against connection task panics.

use std::{net::SocketAddr, sync::Arc};

use async_stream::try_stream;
use cucumber::World;
use tokio::{net::TcpStream, sync::oneshot};
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    connection::ConnectionActor,
    hooks::ProtocolHooks,
    push::PushQueues,
    response::FrameStream,
    server::WireframeServer,
};

#[path = "common/mod.rs"]
mod common;
use common::unused_listener;
#[path = "common/terminator.rs"]
mod terminator;
use terminator::Terminator;

#[derive(Debug)]
struct PanicServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl PanicServer {
    async fn spawn() -> Self {
        let factory = || {
            WireframeApp::new()
                .expect("Failed to create WireframeApp")
                .on_connection_setup(|| async { panic!("boom") })
                .expect("Failed to set connection setup callback")
        };
        let listener = unused_listener();
        let server = WireframeServer::new(factory)
            .workers(1)
            .bind_existing_listener(listener)
            .expect("bind");
        let addr = server.local_addr().expect("Failed to get server address");
        let (tx_shutdown, rx_shutdown) = oneshot::channel();
        let (tx_ready, rx_ready) = oneshot::channel();

        let handle = tokio::spawn(async move {
            server
                .ready_signal(tx_ready)
                .run_with_shutdown(async {
                    let _ = rx_shutdown.await;
                })
                .await
                .expect("Server task failed");
        });
        rx_ready.await.expect("Server did not signal ready");

        Self {
            addr,
            shutdown: Some(tx_shutdown),
            handle,
        }
    }
}

impl Drop for PanicServer {
    fn drop(&mut self) {
        use std::time::Duration;

        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let timeout = Duration::from_secs(5);
        let joined = futures::executor::block_on(tokio::time::timeout(timeout, &mut self.handle));
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("PanicServer task panicked: {e:?}"),
            Err(_) => eprintln!("PanicServer task did not shut down within timeout"),
        }
    }
}

#[derive(Debug, Default, World)]
pub struct PanicWorld {
    server: Option<PanicServer>,
    attempts: usize,
}

impl PanicWorld {
    /// Start a server that panics during connection setup.
    ///
    /// # Panics
    /// Panics if binding the server fails or the server task fails.
    pub async fn start_panic_server(&mut self) { self.server.replace(PanicServer::spawn().await); }

    /// Connect to the running server once.
    ///
    /// # Panics
    /// Panics if the server address is unknown or the connection fails.
    pub async fn connect_once(&mut self) {
        let addr = self.server.as_ref().expect("Server not started").addr;
        TcpStream::connect(addr).await.expect("Failed to connect");
        self.attempts += 1;
    }

    /// Verify both connections succeeded and shut down the server.
    ///
    /// # Panics
    /// Panics if the connection attempts do not match the expected count.
    pub async fn verify_and_shutdown(&mut self) {
        assert_eq!(self.attempts, 2);
        // dropping PanicServer will shut it down
        self.server.take();
        tokio::task::yield_now().await;
    }
}

#[derive(Debug, Default, World)]
pub struct CorrelationWorld {
    cid: u64,
    frames: Vec<Envelope>,
}

impl CorrelationWorld {
    pub fn set_cid(&mut self, cid: u64) { self.cid = cid; }

    #[must_use]
    pub fn cid(&self) -> u64 { self.cid }

    /// Run the connection actor and collect frames for later verification.
    ///
    /// # Panics
    /// Panics if the actor fails to run successfully.
    pub async fn process(&mut self) {
        let cid = self.cid;
        let stream: FrameStream<Envelope> = Box::pin(try_stream! {
            yield Envelope::new(1, Some(cid), vec![1]);
            yield Envelope::new(1, Some(cid), vec![2]);
        });
        let (queues, handle) = PushQueues::builder()
            .high_capacity(1)
            .low_capacity(1)
            .build()
            .unwrap();
        let shutdown = CancellationToken::new();
        let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
        actor.run(&mut self.frames).await.expect("actor run failed");
    }

    /// Verify that all received frames carry the expected correlation id.
    ///
    /// # Panics
    /// Panics if any frame has a `correlation_id` that does not match the
    /// expected value.
    pub fn verify(&self) {
        assert!(
            self.frames
                .iter()
                .all(|f| f.correlation_id() == Some(self.cid))
        );
    }
}

/// Cucumber world that captures frames from a streaming response and verifies
/// that a protocol-provided terminator frame is appended at end-of-stream.
#[derive(Debug, Default, World)]
pub struct StreamEndWorld {
    frames: Vec<u8>,
}

impl StreamEndWorld {
    /// Run the connection actor and record emitted frames.
    ///
    /// # Panics
    /// Panics if the actor fails to run successfully.
    pub async fn process(&mut self) {
        let stream: FrameStream<u8> = Box::pin(try_stream! {
            yield 1u8;
            yield 2u8;
        });

        let (queues, handle) = PushQueues::builder()
            .high_capacity(1)
            .low_capacity(1)
            .build()
            .unwrap();
        let shutdown = CancellationToken::new();
        let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
        let mut actor = ConnectionActor::with_hooks(queues, handle, Some(stream), shutdown, hooks);
        actor.run(&mut self.frames).await.expect("actor run failed");
    }

    /// Verify that a terminator frame was appended to the stream.
    ///
    /// # Panics
    /// Panics if the expected terminator is missing.
    pub fn verify(&self) {
        assert_eq!(self.frames, vec![1, 2, 0]);
    }
}
