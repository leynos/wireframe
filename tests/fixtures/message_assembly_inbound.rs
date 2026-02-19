//! `MessageAssemblyInboundWorld` fixture for inbound assembly integration.

use std::{fmt, future::Future, num::NonZeroUsize, time::Duration};

use futures::SinkExt;
use rstest::fixture;
use tokio::{io::DuplexStream, sync::mpsc, task::JoinHandle, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, Handler, WireframeApp},
    fragment::FragmentationConfig,
    serializer::BincodeSerializer,
    test_helpers::{self, TestAssembler},
};
pub use wireframe_testing::TestResult;

/// Protocol-level message key identifying a logical message stream.
///
/// Wraps a `u64` so step definitions can parse it from feature-file parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageKey(pub u64);

impl From<u64> for MessageKey {
    fn from(value: u64) -> Self { Self(value) }
}

impl From<MessageKey> for u64 {
    fn from(value: MessageKey) -> Self { value.0 }
}

/// Continuation-frame sequence number for ordering validation.
///
/// Wraps a `u32` so step definitions can parse it from feature-file parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameSequence(pub u32);

impl From<u32> for FrameSequence {
    fn from(value: u32) -> Self { Self(value) }
}

impl From<FrameSequence> for u32 {
    fn from(value: FrameSequence) -> Self { value.0 }
}

const ROUTE_ID: u32 = 77;
const CORRELATION_ID: Option<u64> = Some(5);
const BUFFER_CAPACITY: usize = 512;
const PAYLOAD_COLLECT_TIMEOUT_MS: u64 = 200;
const COUNT_COLLECT_TIMEOUT_MS: u64 = 120;

/// Runtime-backed fixture that drives inbound message assembly through
/// `WireframeApp::handle_connection_result`.
pub struct MessageAssemblyInboundWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    client: Option<Framed<DuplexStream, LengthDelimitedCodec>>,
    server: Option<JoinHandle<std::io::Result<()>>>,
    observed_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    observed_payloads: Vec<Vec<u8>>,
    last_send_error: Option<String>,
}

impl fmt::Debug for MessageAssemblyInboundWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageAssemblyInboundWorld")
            .field("client_initialized", &self.client.is_some())
            .field("server_initialized", &self.server.is_some())
            .field("observed_payloads", &self.observed_payloads.len())
            .field("last_send_error", &self.last_send_error)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
struct ContinuationFrameParams<'a> {
    key: u64,
    sequence: u32,
    body: &'a str,
    is_last: bool,
}

impl<'a> ContinuationFrameParams<'a> {
    fn new(key: u64, sequence: u32, body: &'a str, is_last: bool) -> Self {
        Self {
            key,
            sequence,
            body,
            is_last,
        }
    }
}

impl Default for MessageAssemblyInboundWorld {
    fn default() -> Self {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => Self {
                runtime: Some(runtime),
                runtime_error: None,
                client: None,
                server: None,
                observed_rx: None,
                observed_payloads: Vec::new(),
                last_send_error: None,
            },
            Err(err) => Self {
                runtime: None,
                runtime_error: Some(format!("failed to create runtime: {err}")),
                client: None,
                server: None,
                observed_rx: None,
                observed_payloads: Vec::new(),
                last_send_error: None,
            },
        }
    }
}

/// Construct and return a default [`MessageAssemblyInboundWorld`] for BDD
/// test scenarios exercising inbound message assembly integration.
///
/// Injected by rstest into each `#[scenario]` function so that step
/// definitions can drive `WireframeApp::handle_connection_result` through
/// the full assembly pipeline.
// rustfmt collapses simple fixtures into one line, which triggers unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn message_assembly_inbound_world() -> MessageAssemblyInboundWorld {
    MessageAssemblyInboundWorld::default()
}

impl MessageAssemblyInboundWorld {
    fn runtime(&self) -> TestResult<&tokio::runtime::Runtime> {
        self.runtime.as_ref().ok_or_else(|| {
            self.runtime_error
                .clone()
                .unwrap_or_else(|| "runtime unavailable".to_string())
                .into()
        })
    }

    fn block_on<F, T>(&self, future: F) -> TestResult<T>
    where
        F: Future<Output = T>,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("nested Tokio runtime detected in inbound assembly fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    /// Build and start a `WireframeApp` with message assembly enabled.
    ///
    /// Pauses the Tokio clock so that subsequent `wait_millis` calls advance
    /// time deterministically. The handler forwards received payloads to an
    /// internal channel for later assertion.
    ///
    /// # Errors
    ///
    /// Returns an error if the fragmentation config, app builder, or runtime
    /// initialisation fails.
    pub fn start_app(&mut self, timeout_ms: u64) -> TestResult {
        let message_limit =
            NonZeroUsize::new(BUFFER_CAPACITY.saturating_mul(16)).unwrap_or(NonZeroUsize::MIN);
        let config = FragmentationConfig::for_frame_budget(
            BUFFER_CAPACITY,
            message_limit,
            Duration::from_millis(timeout_ms),
        )
        .ok_or("frame budget too small for fragmentation config")?;

        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let handler: Handler<Envelope> = std::sync::Arc::new(move |env: &Envelope| {
            let tx = tx.clone();
            let payload = env.payload_bytes().to_vec();
            Box::pin(async move {
                let _ = tx.send(payload);
            })
        });

        let app: WireframeApp = WireframeApp::new()?
            .buffer_capacity(BUFFER_CAPACITY)
            .fragmentation(Some(config))
            .with_message_assembler(TestAssembler)
            .route(ROUTE_ID, handler)?;

        let codec = app.length_codec();
        let (client_stream, server_stream) = tokio::io::duplex(2048);
        let client = Framed::new(client_stream, codec);

        // Pause the tokio clock so that time-advance steps are deterministic
        // and independent of real wall-clock scheduling.
        self.block_on(async { tokio::time::pause() })?;
        let server = self
            .runtime()?
            .spawn(async move { app.handle_connection_result(server_stream).await });

        self.client = Some(client);
        self.server = Some(server);
        self.observed_rx = Some(rx);
        self.observed_payloads.clear();
        self.last_send_error = None;
        Ok(())
    }

    /// Serialize and send a first frame for the given message `key`.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not initialised or the send fails.
    pub fn send_first_frame(&mut self, key: impl Into<MessageKey>, body: &str) -> TestResult {
        let key = key.into();
        let payload = test_helpers::first_frame_payload(key.0, body.as_bytes(), false, None)?;
        self.send_payload(payload)
    }

    /// Serialize and send a non-final continuation frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not initialised or the send fails.
    pub fn send_continuation_frame(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
    ) -> TestResult {
        let key = key.into();
        let sequence = sequence.into();
        let params = ContinuationFrameParams::new(key.0, sequence.0, body, false);
        self.send_continuation_frame_impl(params)
    }

    /// Serialize and send a final continuation frame that completes assembly.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not initialised or the send fails.
    pub fn send_final_continuation_frame(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
    ) -> TestResult {
        let key = key.into();
        let sequence = sequence.into();
        let params = ContinuationFrameParams::new(key.0, sequence.0, body, true);
        self.send_continuation_frame_impl(params)
    }

    fn send_continuation_frame_impl(&mut self, params: ContinuationFrameParams<'_>) -> TestResult {
        let ContinuationFrameParams {
            key,
            sequence,
            body,
            is_last,
        } = params;
        let payload =
            test_helpers::continuation_frame_payload(key, sequence, body.as_bytes(), is_last)?;
        self.send_payload(payload)
    }

    /// Advance the paused Tokio clock by `millis` milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime is unavailable.
    pub fn wait_millis(&mut self, millis: u64) -> TestResult {
        self.block_on(async { tokio::time::advance(Duration::from_millis(millis)).await })?;
        Ok(())
    }

    /// Assert that the handler has received a payload matching `expected`.
    ///
    /// Collects observed payloads for up to
    /// [`PAYLOAD_COLLECT_TIMEOUT_MS`] before checking.
    ///
    /// # Errors
    ///
    /// Returns an error if the expected payload was not observed.
    pub fn assert_received_payload(&mut self, expected: &str) -> TestResult {
        self.collect_observed_for(Duration::from_millis(PAYLOAD_COLLECT_TIMEOUT_MS))?;
        let expected = expected.as_bytes();
        if self
            .observed_payloads
            .iter()
            .any(|payload| payload.as_slice() == expected)
        {
            return Ok(());
        }

        Err(format!(
            "expected payload {:?} not observed; observed={:?}",
            expected, self.observed_payloads
        )
        .into())
    }

    /// Assert that exactly `expected_count` payloads have been received.
    ///
    /// Collects observed payloads for up to [`COUNT_COLLECT_TIMEOUT_MS`]
    /// before checking.
    ///
    /// # Errors
    ///
    /// Returns an error if the count does not match.
    pub fn assert_received_count(&mut self, expected_count: usize) -> TestResult {
        self.collect_observed_for(Duration::from_millis(COUNT_COLLECT_TIMEOUT_MS))?;
        let actual = self.observed_payloads.len();
        if actual == expected_count {
            return Ok(());
        }
        Err(format!("expected {expected_count} payloads, got {actual}").into())
    }

    /// Assert that no send error has been recorded.
    ///
    /// # Errors
    ///
    /// Returns an error containing the recorded send error message.
    pub fn assert_no_send_error(&self) -> TestResult {
        if self.last_send_error.is_none() {
            return Ok(());
        }
        Err(format!("unexpected send error: {:?}", self.last_send_error).into())
    }

    fn send_payload(&mut self, payload: Vec<u8>) -> TestResult {
        let envelope = Envelope::new(ROUTE_ID, CORRELATION_ID, payload);
        let serializer = BincodeSerializer;
        let frame = serializer.serialize(&envelope)?;

        let mut client = self.client.take().ok_or("client not initialized")?;
        let send_result = self.block_on(async {
            client.send(frame.into()).await?;
            client.flush().await?;
            Ok::<(), std::io::Error>(())
        });

        self.client = Some(client);
        match send_result {
            Ok(Ok(())) => {
                self.last_send_error = None;
                Ok(())
            }
            Ok(Err(err)) => {
                self.last_send_error = Some(err.to_string());
                Err(err.into())
            }
            Err(err) => {
                self.last_send_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    fn collect_observed_for(&mut self, max_wait: Duration) -> TestResult {
        let mut observed_rx = self.observed_rx.take().ok_or("receiver not initialized")?;
        let result = self.block_on(async {
            let mut collected = Vec::new();
            while let Ok(Some(payload)) = timeout(max_wait, observed_rx.recv()).await {
                collected.push(payload);
            }
            collected
        });
        self.observed_rx = Some(observed_rx);
        self.observed_payloads.extend(result?);
        Ok(())
    }
}
