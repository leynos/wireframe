//! Test worlds for Cucumber suites:
//! - Panic resilience during connection setup
//! - Correlation ID propagation across frames
//! - End-of-stream signalling
//! - Channel-backed multi-packet responses (ordered delivery)

use std::{net::SocketAddr, sync::Arc};

use async_stream::try_stream;
use cucumber::World;
use tokio::{net::TcpStream, sync::oneshot};
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet},
    connection::ConnectionActor,
    hooks::ProtocolHooks,
    push::PushQueues,
    response::FrameStream,
    serializer::BincodeSerializer,
    server::WireframeServer,
};
use wireframe_testing::collect_multi_packet;

type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

#[path = "common/mod.rs"]
mod common;
use common::unused_listener;
#[path = "common/terminator.rs"]
mod terminator;
use terminator::Terminator;

#[path = "support.rs"]
mod support;

fn build_small_queues<T: Send + 'static>() -> (PushQueues<T>, wireframe::push::PushHandle<T>) {
    // Prefer the shared builder; use unlimited mode for clarity.
    support::builder::<T>()
        .unlimited()
        .build()
        .expect("failed to build PushQueues")
}

#[derive(Debug)]
struct PanicServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl PanicServer {
    async fn spawn() -> Self {
        let factory = || {
            TestApp::new()
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
        use std::{thread, time::Duration};

        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let timeout = Duration::from_secs(5);
        let handle = self.handle.abort_handle();
        thread::spawn(move || {
            thread::sleep(timeout);
            handle.abort();
        });
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
        let (queues, handle) = build_small_queues::<Envelope>();
        let shutdown = CancellationToken::new();
        let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
        actor.run(&mut self.frames).await.expect("actor run failed");
    }

    /// Verify that all received frames carry the expected correlation ID.
    ///
    /// # Panics
    /// Panics if any frame has a `correlation_id` that does not match `self.cid`.
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

        let (queues, handle) = build_small_queues::<u8>();
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

#[derive(Debug, Default, World)]
pub struct MultiPacketWorld {
    messages: Vec<u8>,
}

impl MultiPacketWorld {
    /// Helper method to process messages through a multi-packet response.
    ///
    /// # Panics
    /// Panics if sending to the channel fails.
    async fn process_messages(&mut self, messages: &[u8]) {
        self.messages.clear();
        let (tx, ch_rx) = tokio::sync::mpsc::channel(4);
        for &msg in messages {
            tx.send(msg).await.expect("send");
        }
        drop(tx);
        let resp: wireframe::Response<u8, ()> = wireframe::Response::MultiPacket(ch_rx);
        self.messages = collect_multi_packet(resp).await;
    }

    /// Send messages through a multi-packet response and record them.
    ///
    /// # Panics
    /// Panics if sending to the channel fails.
    pub async fn process(&mut self) { self.process_messages(&[1, 2, 3]).await; }

    /// Record zero messages from a closed channel.
    ///
    /// # Panics
    /// Does not panic.
    pub async fn process_empty(&mut self) { self.process_messages(&[]).await; }

    /// Verify that no messages were received.
    ///
    /// # Panics
    /// Panics if any messages are present.
    pub fn verify_empty(&self) {
        assert!(self.messages.is_empty());
    }

    /// Verify messages were received in order.
    ///
    /// # Panics
    ///
    /// Panics if the messages are not in the expected order.
    pub fn verify(&self) {
        assert_eq!(self.messages, vec![1, 2, 3]);
    }
}
