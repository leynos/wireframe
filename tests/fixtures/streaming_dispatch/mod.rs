//! BDD fixture for streaming request-body dispatch.
//!
//! Drives a real `WireframeApp` over an in-process duplex stream so scenarios
//! can observe what a streaming handler receives, when a parked buffered
//! message is dispatched, and which reply frames reach the client.

mod asserts;

use std::{fmt, future::Future, io, num::NonZeroUsize, sync::Arc, time::Duration};

use futures::StreamExt;
use rstest::fixture;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Semaphore, mpsc},
    task::JoinHandle,
    time::timeout,
};
use wireframe::{
    app::{Envelope, Handler, StreamingHandler, WireframeApp},
    fragment::FragmentationConfig,
    message_assembler::{FrameSequence, MessageKey},
    request::{RequestBodyStream, RequestParts},
    serializer::{BincodeSerializer, Serializer},
    test_helpers,
};
pub use wireframe_testing::TestResult;

/// Route identifier for the streaming handler.
pub const STREAM_ROUTE: u32 = 90;
/// Route identifier for the buffered handler.
pub const BUFFERED_ROUTE: u32 = 91;
/// Correlation identifier carried by every test envelope.
const CORRELATION_ID: Option<u64> = Some(5);
/// Per-connection frame budget.
const BUFFER_CAPACITY: usize = 512;
/// Inbound read timeout, short enough that a paused header is noticed quickly.
const READ_TIMEOUT_MS: u64 = 100;
/// Longest a single assertion waits for handler activity.
const COLLECT_TIMEOUT_MS: u64 = 400;
/// Reply payload returned once a streaming body has been consumed.
const STREAM_REPLY: &[u8] = b"stream-reply";
/// Reply payload returned by a handler that abandons its body.
const ABANDONED_REPLY: &[u8] = b"abandoned";

/// What the streaming handler does with its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlerMode {
    /// Read every chunk and reply when the body ends.
    Consume,
    /// Wait for a release signal before reading any chunk.
    Hold,
    /// Return immediately without reading the body.
    Abandon,
}

/// One observation reported by a route handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DispatchEvent {
    /// The streaming handler received a body chunk.
    Chunk {
        /// Chunk bytes.
        body: Vec<u8>,
    },
    /// The streaming handler's body ended with an error.
    Error {
        /// Error kind reported on the body stream.
        kind: io::ErrorKind,
    },
    /// The streaming handler read the body to its end.
    Completed,
    /// The buffered handler received a complete payload.
    Buffered {
        /// Dispatched payload bytes.
        body: Vec<u8>,
    },
}

/// BDD world that drives streaming dispatch over a duplex connection.
pub struct StreamingDispatchWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    client: Option<DuplexStream>,
    server: Option<JoinHandle<io::Result<()>>>,
    events: Option<mpsc::UnboundedReceiver<DispatchEvent>>,
    observed: Vec<DispatchEvent>,
    gate: Option<Arc<Semaphore>>,
    pending: Vec<u8>,
    replies: Vec<Vec<u8>>,
    server_outcome: Option<String>,
}

impl fmt::Debug for StreamingDispatchWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamingDispatchWorld")
            .field("client_initialized", &self.client.is_some())
            .field("observed_events", &self.observed.len())
            .field("pending_bytes", &self.pending.len())
            .field("replies", &self.replies.len())
            .field("server_outcome", &self.server_outcome)
            .finish_non_exhaustive()
    }
}

impl Default for StreamingDispatchWorld {
    fn default() -> Self {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => Self::with_runtime(runtime),
            Err(error) => {
                let mut world = Self::with_runtime_placeholder();
                world.runtime_error = Some(format!("failed to create runtime: {error}"));
                world
            }
        }
    }
}

impl StreamingDispatchWorld {
    fn with_runtime(runtime: tokio::runtime::Runtime) -> Self {
        Self {
            runtime: Some(runtime),
            runtime_error: None,
            client: None,
            server: None,
            events: None,
            observed: Vec::new(),
            gate: None,
            pending: Vec::new(),
            replies: Vec::new(),
            server_outcome: None,
        }
    }

    fn with_runtime_placeholder() -> Self {
        Self {
            runtime: None,
            runtime_error: None,
            client: None,
            server: None,
            events: None,
            observed: Vec::new(),
            gate: None,
            pending: Vec::new(),
            replies: Vec::new(),
            server_outcome: None,
        }
    }

    fn runtime(&self) -> TestResult<&tokio::runtime::Runtime> {
        self.runtime.as_ref().ok_or_else(|| {
            self.runtime_error
                .clone()
                .unwrap_or_else(|| "runtime unavailable".to_owned())
                .into()
        })
    }

    fn block_on<F, T>(&self, future: F) -> TestResult<T>
    where
        F: Future<Output = T>,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("nested Tokio runtime detected in streaming dispatch fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    /// Start the app with the given handler mode and optional assembly timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if the app, its routes, or the runtime cannot be built.
    #[expect(
        deprecated,
        reason = "fixture covers the legacy builder connection API"
    )]
    pub fn start(&mut self, mode: HandlerMode, timeout_ms: Option<u64>) -> TestResult {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let gate = Arc::new(Semaphore::new(0));

        let streaming = build_streaming_handler(mode, gate.clone(), events_tx.clone());
        let buffered = build_buffered_handler(events_tx);

        let mut app: WireframeApp = WireframeApp::new()?
            .buffer_capacity(BUFFER_CAPACITY)
            .read_timeout_ms(READ_TIMEOUT_MS)
            .with_message_assembler(test_helpers::TestAssembler);

        if let Some(millis) = timeout_ms {
            let message_limit =
                NonZeroUsize::new(BUFFER_CAPACITY.saturating_mul(16)).unwrap_or(NonZeroUsize::MIN);
            let config = FragmentationConfig::for_frame_budget(
                BUFFER_CAPACITY,
                message_limit,
                Duration::from_millis(millis),
            )
            .ok_or("frame budget too small for fragmentation config")?;
            app = app.fragmentation(Some(config));
        }

        app = app
            .route(BUFFERED_ROUTE, buffered)?
            .streaming_route(STREAM_ROUTE, streaming)?;

        let (client, server_stream) = tokio::io::duplex(4096);
        let server = self
            .runtime()?
            .spawn(async move { app.handle_connection_result(server_stream).await });

        self.client = Some(client);
        self.server = Some(server);
        self.events = Some(events_rx);
        self.gate = Some(gate);
        self.observed.clear();
        self.pending.clear();
        self.replies.clear();
        self.server_outcome = None;
        Ok(())
    }

    /// Start with a handler that consumes its body.
    ///
    /// # Errors
    ///
    /// See [`start`](Self::start).
    pub fn start_consuming(&mut self) -> TestResult { self.start(HandlerMode::Consume, None) }

    /// Start with a handler that waits for a release signal.
    ///
    /// # Errors
    ///
    /// See [`start`](Self::start).
    pub fn start_holding(&mut self) -> TestResult { self.start(HandlerMode::Hold, None) }

    /// Start with a handler that returns without reading its body.
    ///
    /// # Errors
    ///
    /// See [`start`](Self::start).
    pub fn start_abandoning(&mut self) -> TestResult { self.start(HandlerMode::Abandon, None) }

    /// Start with a short assembly timeout.
    ///
    /// # Errors
    ///
    /// See [`start`](Self::start).
    pub fn start_with_timeout(&mut self, millis: u64) -> TestResult {
        self.start(HandlerMode::Consume, Some(millis))
    }

    /// Send a streaming first frame, optionally declaring the body length.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_streaming_first(
        &mut self,
        key: impl Into<MessageKey>,
        body: &str,
        total: Option<u32>,
    ) -> TestResult {
        let payload = test_helpers::first_frame_payload(key.into(), body.as_bytes(), false, total)?;
        self.send_message(STREAM_ROUTE, payload)
    }

    /// Send a streaming continuation frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_streaming_continuation(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
        is_last: bool,
    ) -> TestResult {
        let payload = test_helpers::continuation_frame_payload(
            key.into(),
            sequence.into(),
            body.as_bytes(),
            is_last,
        )?;
        self.send_message(STREAM_ROUTE, payload)
    }

    /// Send a buffered first frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_buffered_first(
        &mut self,
        key: impl Into<MessageKey>,
        body: &str,
        is_last: bool,
    ) -> TestResult {
        let total = is_last.then(|| u32::try_from(body.len()).unwrap_or(u32::MAX));
        let payload =
            test_helpers::first_frame_payload(key.into(), body.as_bytes(), is_last, total)?;
        self.send_message(BUFFERED_ROUTE, payload)
    }

    /// Send a buffered continuation frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_buffered_continuation(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
        is_last: bool,
    ) -> TestResult {
        let payload = test_helpers::continuation_frame_payload(
            key.into(),
            sequence.into(),
            body.as_bytes(),
            is_last,
        )?;
        self.send_message(BUFFERED_ROUTE, payload)
    }

    /// Send the first part of a complete buffered message, retaining the rest.
    ///
    /// The split lands inside the transport length prefix so the server sees a
    /// paused frame header. The runtime is then given time to reach a read
    /// timeout with the partial frame buffered.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be built or sent.
    pub fn send_split_buffered_first(
        &mut self,
        key: impl Into<MessageKey>,
        body: &str,
    ) -> TestResult {
        let total = u32::try_from(body.len()).unwrap_or(u32::MAX);
        let payload =
            test_helpers::first_frame_payload(key.into(), body.as_bytes(), true, Some(total))?;
        let framed = transport_frame(&self.encode_message(BUFFERED_ROUTE, &payload)?)?;
        let split = framed.len().min(2);
        let (head, tail) = framed.split_at(split);
        self.write_raw(head)?;
        self.pending = tail.to_vec();
        self.wait_millis(3 * READ_TIMEOUT_MS / 2)
    }

    /// Write the retained remainder of a split frame.
    ///
    /// # Errors
    ///
    /// Returns an error if nothing was retained or the write fails.
    pub fn send_pending_remainder(&mut self) -> TestResult {
        let remainder = std::mem::take(&mut self.pending);
        if remainder.is_empty() {
            return Err("no partial frame is pending".into());
        }
        self.write_raw(&remainder)
    }

    /// Release a handler waiting in [`HandlerMode::Hold`].
    pub fn release_handler(&mut self) {
        if let Some(gate) = self.gate.as_ref() {
            gate.add_permits(1);
        }
    }

    /// Let wall-clock time pass while the connection runtime keeps running.
    ///
    /// Assembly expiry uses `std::time::Instant`, so the wait must be real
    /// time rather than an advanced virtual clock.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime is unavailable.
    pub fn wait_millis(&mut self, millis: u64) -> TestResult {
        self.block_on(async move { tokio::time::sleep(Duration::from_millis(millis)).await })
    }

    /// Close the client side and await the server task.
    ///
    /// # Errors
    ///
    /// Returns an error if the server task cannot be awaited.
    pub fn close_connection(&mut self) -> TestResult {
        self.client = None;
        let Some(server) = self.server.take() else {
            return Err("server not initialized".into());
        };
        let outcome = self.block_on(async { timeout(Duration::from_secs(2), server).await });
        match outcome? {
            Ok(Ok(Ok(()))) => {
                self.server_outcome = Some("ok".to_owned());
                self.collect_events_for(Duration::from_millis(COLLECT_TIMEOUT_MS))
            }
            Ok(Ok(Err(error))) => {
                self.server_outcome = Some(format!("connection error: {error}"));
                Ok(())
            }
            Ok(Err(join_error)) => {
                self.server_outcome = Some(format!("server task failed: {join_error}"));
                Ok(())
            }
            Err(_) => Err("server did not finish within two seconds".into()),
        }
    }

    fn take_client(&mut self) -> Option<DuplexStream> { self.client.take() }

    fn restore_client(&mut self, client: DuplexStream) { self.client = Some(client); }

    fn server_is_open(&self) -> bool { self.server.is_some() }

    fn server_outcome_text(&self) -> Option<&str> { self.server_outcome.as_deref() }

    fn send_message(&mut self, route: u32, payload: Vec<u8>) -> TestResult {
        let framed = transport_frame(&self.encode_message(route, &payload)?)?;
        self.write_raw(&framed)
    }

    fn encode_message(&self, route: u32, payload: &[u8]) -> TestResult<Vec<u8>> {
        let envelope = Envelope::new(route, CORRELATION_ID, payload.to_vec());
        Ok(BincodeSerializer.serialize(&envelope)?)
    }

    /// Write raw transport bytes and let the connection runtime make progress.
    fn write_raw(&mut self, bytes: &[u8]) -> TestResult {
        let mut client = self.client.take().ok_or("client not initialized")?;
        let result = self.block_on(async {
            client.write_all(bytes).await?;
            client.flush().await
        });
        self.client = Some(client);
        result??;
        self.block_on(async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        })?;
        Ok(())
    }

    fn collect_events_for(&mut self, wait: Duration) -> TestResult {
        let mut events = self.events.take().ok_or("event receiver not initialized")?;
        let result = self.block_on(async {
            let deadline = tokio::time::Instant::now() + wait;
            let mut collected = Vec::new();
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match timeout(remaining, events.recv()).await {
                    Ok(Some(event)) => collected.push(event),
                    Ok(None) | Err(_) => break,
                }
            }
            collected
        });
        self.events = Some(events);
        self.observed.extend(result?);
        Ok(())
    }
}

/// Prefix serialized envelope bytes with their transport length.
fn transport_frame(envelope: &[u8]) -> TestResult<Vec<u8>> {
    let len = u32::try_from(envelope.len()).map_err(|_| "frame too large")?;
    let mut framed = Vec::with_capacity(envelope.len() + 4);
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(envelope);
    Ok(framed)
}

/// Construct the streaming dispatch world used by these scenarios.
///
/// # Errors
///
/// Returns an error if the scenario runtime is unavailable.
#[rustfmt::skip]
#[fixture]
pub fn streaming_dispatch_world() -> TestResult<StreamingDispatchWorld> {
    let world = StreamingDispatchWorld::default();
    world.runtime()?;
    Ok(world)
}

/// Build the streaming route handler for the given mode.
fn build_streaming_handler(
    mode: HandlerMode,
    gate: Arc<Semaphore>,
    events: mpsc::UnboundedSender<DispatchEvent>,
) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        let gate = Arc::clone(&gate);
        let events = events.clone();
        Box::pin(async move {
            if mode == HandlerMode::Abandon {
                return Some(ABANDONED_REPLY.to_vec());
            }
            if mode == HandlerMode::Hold {
                let _permit = gate.acquire().await;
            }
            consume_body(body, &events).await;
            Some(STREAM_REPLY.to_vec())
        })
    })
}

/// Read a body to its end, reporting each chunk and how it ended.
///
/// The first error terminates the body, so a completed body and an errored
/// body are mutually exclusive observations.
async fn consume_body(mut body: RequestBodyStream, events: &mpsc::UnboundedSender<DispatchEvent>) {
    while let Some(item) = body.next().await {
        match item {
            Ok(chunk) => {
                let _sent = events.send(DispatchEvent::Chunk {
                    body: chunk.to_vec(),
                });
            }
            Err(error) => {
                let _sent = events.send(DispatchEvent::Error { kind: error.kind() });
                return;
            }
        }
    }
    let _sent = events.send(DispatchEvent::Completed);
}

/// Build the buffered route handler, which reports its payload.
fn build_buffered_handler(events: mpsc::UnboundedSender<DispatchEvent>) -> Handler<Envelope> {
    Arc::new(move |envelope: &Envelope| {
        let events = events.clone();
        let body = envelope.payload_bytes().to_vec();
        Box::pin(async move {
            let _sent = events.send(DispatchEvent::Buffered { body });
        })
    })
}

/// Read one length-delimited reply frame from the client side.
async fn read_frame(client: &mut DuplexStream) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    client.read_exact(&mut len).await?;
    let mut payload = vec![0u8; u32::from_be_bytes(len) as usize];
    client.read_exact(&mut payload).await?;
    Ok(payload)
}
