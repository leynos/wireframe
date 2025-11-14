#![cfg(not(loom))]
//! Test worlds for Cucumber suites:
//! - Panic resilience during connection setup
//! - Correlation ID propagation across frames
//! - End-of-stream signalling
//! - Channel-backed multi-packet responses (ordered delivery)

use std::{future::Future, marker::PhantomData, net::SocketAddr, ptr, sync::Arc};

use async_stream::try_stream;
use cucumber::World;
use log::Level;
use tokio::{
    net::TcpStream,
    sync::{
        mpsc::{self, error::TrySendError},
        oneshot,
    },
};
use tokio_util::sync::CancellationToken;
use wireframe::{
    Response,
    app::{Envelope, Packet},
    connection::{ConnectionActor, test_support::ActorHarness},
    fragment::{
        FragmentError,
        FragmentHeader,
        FragmentIndex,
        FragmentSeries,
        FragmentStatus,
        MessageId,
    },
    hooks::ProtocolHooks,
    push::PushQueues,
    response::FrameStream,
    serializer::BincodeSerializer,
    server::WireframeServer,
};
use wireframe_testing::{LoggerHandle, logger};

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
    expected: Option<u64>,
    frames: Vec<Envelope>,
}

impl CorrelationWorld {
    pub fn set_expected(&mut self, expected: Option<u64>) { self.expected = expected; }

    #[must_use]
    pub fn expected(&self) -> Option<u64> { self.expected }

    /// Run the connection actor and collect frames for later verification.
    ///
    /// # Panics
    /// Panics if the actor fails to run successfully.
    pub async fn process(&mut self) {
        let cid = self
            .expected
            .expect("streaming scenario requires a correlation id");
        let stream: FrameStream<Envelope> = Box::pin(try_stream! {
            yield Envelope::new(1, Some(cid), vec![1]);
            yield Envelope::new(1, Some(cid), vec![2]);
        });
        let (queues, handle) = build_small_queues::<Envelope>();
        let shutdown = CancellationToken::new();
        let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
        actor.run(&mut self.frames).await.expect("actor run failed");
    }

    /// Run the connection actor for a multi-packet channel and collect frames.
    ///
    /// # Panics
    /// Panics if sending to the channel or running the actor fails.
    pub async fn process_multi(&mut self) {
        let expected = self.expected;
        let (tx, rx) = mpsc::channel(4);
        tx.send(Envelope::new(1, None, vec![1]))
            .await
            .expect("send frame");
        tx.send(Envelope::new(1, Some(99), vec![2]))
            .await
            .expect("send frame");
        drop(tx);

        let (queues, handle) = build_small_queues::<Envelope>();
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<Envelope, ()> =
            ConnectionActor::new(queues, handle, None, shutdown);
        actor.set_multi_packet_with_correlation(Some(rx), expected);
        actor.run(&mut self.frames).await.expect("actor run failed");
    }

    /// Verify that all received frames respect the configured correlation expectation.
    ///
    /// # Panics
    /// Panics if any frame violates the stored correlation expectation.
    pub fn verify(&self) {
        match self.expected {
            Some(cid) => {
                assert!(self.frames.iter().all(|f| f.correlation_id() == Some(cid)));
            }
            None => {
                assert!(self.frames.iter().all(|f| f.correlation_id().is_none()));
            }
        }
    }
}

/// Cucumber world that captures frames from a streaming response and verifies
/// that a protocol-provided terminator frame is appended at end-of-stream.
#[derive(Debug, Default, World)]
pub struct StreamEndWorld {
    frames: Vec<u8>,
    logs: Vec<(Level, String)>,
}

struct StreamEndTestGuard<'a> {
    world: *mut StreamEndWorld,
    logger: LoggerHandle,
    _marker: PhantomData<&'a mut StreamEndWorld>,
}

impl<'a> StreamEndTestGuard<'a> {
    fn new(world: &'a mut StreamEndWorld) -> Self {
        let logger = world.prepare_test();
        Self {
            world: ptr::from_mut(world),
            logger,
            _marker: PhantomData,
        }
    }

    fn run<F, R>(mut self, f: F) -> R
    where
        F: FnOnce(&'a mut StreamEndWorld, &mut LoggerHandle) -> R,
    {
        // Safety: the guard guarantees exclusive access to the world while the closure runs.
        let world = unsafe { &mut *self.world };
        f(world, &mut self.logger)
    }

    async fn run_async<F, Fut, R>(mut self, f: F) -> R
    where
        F: FnOnce(&'a mut StreamEndWorld, &mut LoggerHandle) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        // Safety: the guard guarantees exclusive access to the world while the future runs.
        let world = unsafe { &mut *self.world };
        f(world, &mut self.logger).await
    }
}

impl Drop for StreamEndTestGuard<'_> {
    fn drop(&mut self) {
        // Safety: the guard created the pointer from a unique mutable reference.
        unsafe {
            (&mut *self.world).finalize_test(&mut self.logger);
        }
    }
}

impl StreamEndWorld {
    fn prepare_test(&mut self) -> LoggerHandle {
        self.frames.clear();
        self.logs.clear();
        let mut logger = logger();
        logger.clear();
        logger
    }

    fn finalize_test(&mut self, logger: &mut LoggerHandle) { self.capture_logs(logger); }

    fn with_sync_test<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Self, &mut LoggerHandle) -> R,
    {
        StreamEndTestGuard::new(self).run(f)
    }

    async fn with_async_test<'a, F, Fut, R>(&'a mut self, f: F) -> R
    where
        F: FnOnce(&'a mut Self, &mut LoggerHandle) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        StreamEndTestGuard::new(self).run_async(f).await
    }

    /// Run the connection actor and record emitted frames.
    ///
    /// # Panics
    /// Panics if the actor fails to run successfully.
    pub async fn process(&mut self) {
        self.with_async_test(|this, _log| async {
            let stream: FrameStream<u8> = Box::pin(try_stream! {
                yield 1u8;
                yield 2u8;
            });

            let (queues, handle) = build_small_queues::<u8>();
            let shutdown = CancellationToken::new();
            let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
            let mut actor =
                ConnectionActor::with_hooks(queues, handle, Some(stream), shutdown, hooks);
            actor.run(&mut this.frames).await.expect("actor run failed");
        })
        .await;
    }

    /// Run the connection actor with a multi-packet channel and record emitted frames.
    ///
    /// # Panics
    /// Panics if sending to the channel or running the actor fails.
    pub async fn process_multi(&mut self) {
        self.with_async_test(|this, _log| async {
            let (tx, rx) = mpsc::channel(4);
            tx.send(1u8).await.expect("send frame");
            tx.send(2u8).await.expect("send frame");
            drop(tx);

            let (queues, handle) = build_small_queues::<u8>();
            let shutdown = CancellationToken::new();
            let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
            let mut actor = ConnectionActor::with_hooks(queues, handle, None, shutdown, hooks);
            actor.set_multi_packet(Some(rx));
            actor.run(&mut this.frames).await.expect("actor run failed");
        })
        .await;
    }

    fn capture_logs(&mut self, logger: &mut LoggerHandle) {
        while let Some(record) = logger.pop() {
            self.logs.push((record.level(), record.args().to_string()));
        }
    }

    fn closure_log(&self) -> Option<&(Level, String)> {
        self.logs
            .iter()
            .rev()
            .find(|(_, message)| message.contains("multi-packet stream closed"))
    }

    /// Simulate a disconnected multi-packet channel by dropping the sender before draining.
    ///
    /// # Panics
    /// Panics if creating the harness or sending frames fails.
    pub fn process_multi_disconnect(&mut self) {
        self.with_sync_test(|this, log| {
            let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
            let mut harness = ActorHarness::new_with_state(hooks, false, true)
                .expect("failed to create ActorHarness");
            let (tx, rx) = mpsc::channel(4);
            tx.try_send(1u8).expect("send frame");
            tx.try_send(2u8).expect("send frame");
            harness
                .actor_mut()
                .set_multi_packet_with_correlation(Some(rx), Some(42));
            drop(tx);
            log.clear();
            while harness.try_drain_multi() {}
            this.frames.clone_from(&harness.out);
        });
    }

    /// Trigger shutdown handling on a multi-packet channel without emitting a terminator.
    ///
    /// # Panics
    /// Panics if creating the harness fails.
    pub fn process_multi_shutdown(&mut self) {
        self.with_sync_test(|this, log| {
            let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
            let mut harness = ActorHarness::new_with_state(hooks, false, true)
                .expect("failed to create ActorHarness");
            let (_tx, rx) = mpsc::channel(4);
            harness
                .actor_mut()
                .set_multi_packet_with_correlation(Some(rx), Some(77));
            log.clear();
            harness.start_shutdown();
            this.frames.clone_from(&harness.out);
        });
    }

    /// Verify that a terminator frame was appended to the stream.
    ///
    /// # Panics
    /// Panics if the expected terminator is missing.
    pub fn verify(&self) {
        assert_eq!(self.frames, vec![1, 2, 0]);
    }

    /// Verify that a multi-packet terminator frame was appended to the stream.
    ///
    /// # Panics
    /// Panics if the expected terminator is missing.
    pub fn verify_multi(&self) {
        assert_eq!(self.frames, vec![1, 2, 0]);
    }

    /// Verify that no terminator frame was emitted.
    ///
    /// # Panics
    /// Panics if a terminator frame is present.
    pub fn verify_no_multi(&self) {
        assert!(
            self.frames.iter().all(|&frame| frame != 0),
            "unexpected terminator frame present",
        );
    }

    /// Verify the logged multi-packet termination reason.
    ///
    /// # Panics
    /// Panics if the closure log is missing or contains unexpected details.
    pub fn verify_reason(&self, expected: &str) {
        let (level, message) = self
            .closure_log()
            .expect("multi-packet closure log missing");
        let expected_level = match expected {
            "disconnected" => Level::Warn,
            _ => Level::Info,
        };
        assert_eq!(
            *level, expected_level,
            "unexpected log level: message={message}",
        );
        assert!(
            message.contains(&format!("reason={expected}")),
            "closure log missing reason: message={message}",
        );
    }
}

#[derive(Debug, Default, World)]
pub struct MultiPacketWorld {
    messages: Vec<u8>,
    overflow_error: bool,
}

impl MultiPacketWorld {
    async fn collect_frames_from(rx: mpsc::Receiver<u8>) -> Vec<u8> {
        let (queues, handle) = build_small_queues::<u8>();
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<_, ()> =
            ConnectionActor::new(queues, handle, None, shutdown);
        actor.set_multi_packet(Some(rx));

        let mut frames = Vec::new();
        actor.run(&mut frames).await.expect("actor run failed");
        frames
    }

    /// Helper method to process messages through a multi-packet response built
    /// via [`Response::with_channel`].
    ///
    /// # Panics
    /// Panics if spawning or joining the producer task fails.
    async fn process_messages(&mut self, messages: &[u8]) {
        let (sender, response): (mpsc::Sender<u8>, Response<u8, ()>) = Response::with_channel(4);
        let Response::MultiPacket(rx) = response else {
            panic!("helper did not return a MultiPacket response");
        };

        let payload = messages.to_vec();
        let producer = tokio::spawn(async move {
            for msg in payload {
                if sender.send(msg).await.is_err() {
                    return;
                }
            }
            drop(sender);
        });

        let frames = Self::collect_frames_from(rx).await;
        producer.await.expect("producer task panicked");
        self.messages = frames;
        self.overflow_error = false;
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

    /// Attempt to send more messages than the channel can buffer at once.
    ///
    /// # Panics
    /// Panics if sending to the channel fails unexpectedly or the producer task panics.
    pub async fn process_overflow(&mut self) {
        let (sender, response): (mpsc::Sender<u8>, Response<u8, ()>) = Response::with_channel(1);
        let Response::MultiPacket(rx) = response else {
            panic!("helper did not return a MultiPacket response");
        };

        sender.try_send(1).expect("send initial frame");
        let overflow_error = matches!(sender.try_send(2), Err(TrySendError::Full(2)));

        let producer = tokio::spawn(async move {
            sender
                .send(2)
                .await
                .expect("send follow-up frame after draining");
            drop(sender);
        });

        let frames = Self::collect_frames_from(rx).await;
        producer.await.expect("producer task panicked");

        self.messages = frames;
        self.overflow_error = overflow_error;
    }

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

    /// Verify that the channel enforced back-pressure.
    ///
    /// # Panics
    /// Panics if no overflow occurred or if the expected messages are missing.
    pub fn verify_overflow(&self) {
        assert!(
            self.overflow_error,
            "expected overflow error when channel capacity was exceeded",
        );
        assert_eq!(self.messages, vec![1, 2]);
    }
}

#[derive(Debug, Default, World)]
pub struct FragmentWorld {
    series: Option<FragmentSeries>,
    last_result: Option<Result<FragmentStatus, FragmentError>>,
}

impl FragmentWorld {
    /// Start tracking a new logical message.
    pub fn start_series(&mut self, message_id: u64) {
        self.series = Some(FragmentSeries::new(MessageId::new(message_id)));
        self.last_result = None;
    }

    /// Feed a fragment that references the currently tracked message.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn accept_fragment(&mut self, index: u32, is_last: bool) {
        let message = self
            .series
            .as_ref()
            .expect("fragment series not initialised")
            .message_id()
            .get();
        self.accept_fragment_from(message, index, is_last);
    }

    /// Feed a fragment for an explicit message identifier.
    ///
    /// # Panics
    /// Panics if [`start_series`] has not been called.
    pub fn accept_fragment_from(&mut self, message: u64, index: u32, is_last: bool) {
        let header =
            FragmentHeader::new(MessageId::new(message), FragmentIndex::new(index), is_last);
        let series = self
            .series
            .as_mut()
            .expect("fragment series not initialised");
        self.last_result = Some(series.accept(header));
    }

    /// Return the most recent fragment outcome.
    ///
    /// # Panics
    /// Panics if no fragment has been processed yet.
    fn last_result(&self) -> &Result<FragmentStatus, FragmentError> {
        self.last_result
            .as_ref()
            .expect("no fragment processed yet")
    }

    /// Assert that the latest fragment completed the logical message.
    ///
    /// # Panics
    /// Panics if no fragment was processed or if the fragment failed to
    /// complete the message.
    pub fn assert_completion(&self) {
        match self.last_result() {
            Ok(FragmentStatus::Complete) => {}
            Ok(status) => panic!("unexpected status: {status:?}"),
            Err(err) => panic!("expected completion but got error: {err}"),
        }
        let series = self.series.as_ref().expect("series missing");
        assert!(series.is_complete(), "series should be marked complete");
    }

    /// Assert that the latest fragment failed due to an index mismatch.
    ///
    /// # Panics
    /// Panics if no fragment was processed or if the fragment failed for some
    /// other reason.
    pub fn assert_index_mismatch(&self) {
        let err = match self.last_result() {
            Err(err) => err,
            Ok(status) => panic!("expected error but received {status:?}"),
        };
        assert!(
            matches!(err, FragmentError::IndexMismatch { .. }),
            "expected index mismatch, got {err}"
        );
    }

    /// Assert that the latest fragment failed because the message identifier
    /// did not match the tracked series.
    ///
    /// # Panics
    /// Panics if no fragment was processed or if the fragment failed for a
    /// different reason.
    pub fn assert_message_mismatch(&self) {
        let err = match self.last_result() {
            Err(err) => err,
            Ok(status) => panic!("expected error but received {status:?}"),
        };
        assert!(
            matches!(err, FragmentError::MessageMismatch { .. }),
            "expected message mismatch, got {err}"
        );
    }
}
