//! Behavioural fixture for derived memory budget default scenarios.

use std::{fmt, future::Future, num::NonZeroUsize, time::Duration};

use futures::SinkExt;
use rstest::fixture;
use tokio::{io::DuplexStream, sync::mpsc, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{BudgetBytes, Envelope, Handler, MemoryBudgets, WireframeApp},
    fragment::FragmentationConfig,
    serializer::{BincodeSerializer, Serializer},
    test_helpers::{self, TestAssembler},
};
pub use wireframe_testing::TestResult;

/// Parsed as "`per_message` / `per_connection` / `in_flight`".
#[derive(Clone, Copy, Debug)]
pub struct ExplicitBudgetConfig {
    pub per_message: usize,
    pub per_connection: usize,
    pub in_flight: usize,
}

impl std::str::FromStr for ExplicitBudgetConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut values = s.split('/').map(str::trim);
        let per_message = values
            .next()
            .filter(|value| !value.is_empty())
            .ok_or("missing per_message")?;
        let per_connection = values
            .next()
            .filter(|value| !value.is_empty())
            .ok_or("missing per_connection")?;
        let in_flight = values
            .next()
            .filter(|value| !value.is_empty())
            .ok_or("missing in_flight")?;
        if values.next().is_some() {
            return Err("unexpected trailing segments".to_string());
        }
        Ok(Self {
            per_message: per_message
                .parse()
                .map_err(|error| format!("per_message: {error}"))?,
            per_connection: per_connection
                .parse()
                .map_err(|error| format!("per_connection: {error}"))?,
            in_flight: in_flight
                .parse()
                .map_err(|error| format!("in_flight: {error}"))?,
        })
    }
}

const ROUTE_ID: u32 = 91;
const CORRELATION_ID: Option<u64> = Some(20);
const SPIN_ATTEMPTS: usize = 64;

/// Runtime-backed fixture that drives inbound assembly with derived (or
/// explicit) memory budgets and validates protection tier behaviour.
pub struct DerivedMemoryBudgetsWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    client: Option<Framed<DuplexStream, LengthDelimitedCodec>>,
    server: Option<JoinHandle<std::io::Result<()>>>,
    observed_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    observed_payloads: Vec<Vec<u8>>,
    last_send_error: Option<String>,
    connection_error: Option<String>,
}

impl DerivedMemoryBudgetsWorld {
    fn with_runtime(
        runtime: Option<tokio::runtime::Runtime>,
        runtime_error: Option<String>,
    ) -> Self {
        Self {
            runtime,
            runtime_error,
            client: None,
            server: None,
            observed_rx: None,
            observed_payloads: Vec::new(),
            last_send_error: None,
            connection_error: None,
        }
    }
}

impl Default for DerivedMemoryBudgetsWorld {
    fn default() -> Self {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => Self::with_runtime(Some(runtime), None),
            Err(error) => {
                Self::with_runtime(None, Some(format!("failed to create runtime: {error}")))
            }
        }
    }
}

impl fmt::Debug for DerivedMemoryBudgetsWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DerivedMemoryBudgetsWorld")
            .field("client_initialized", &self.client.is_some())
            .field("server_initialized", &self.server.is_some())
            .field("observed_payloads", &self.observed_payloads.len())
            .field("last_send_error", &self.last_send_error)
            .field("connection_error", &self.connection_error)
            .finish_non_exhaustive()
    }
}

/// Construct the default world used by derived memory budget BDD tests.
#[fixture]
pub fn derived_memory_budgets_world() -> DerivedMemoryBudgetsWorld {
    DerivedMemoryBudgetsWorld::default()
}

impl DerivedMemoryBudgetsWorld {
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
            return Err("nested Tokio runtime detected in derived-budget fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    /// Derive a fragmentation config from the app's clamped frame length.
    ///
    /// Uses the codec's actual `max_frame_length()` (post-clamping) to keep
    /// the test fixture aligned with production derivation behaviour.
    fn fragmentation_from_app(app: &WireframeApp) -> TestResult<FragmentationConfig> {
        let clamped = app.length_codec().max_frame_length();
        let fragment_limit = NonZeroUsize::new(clamped.saturating_mul(16))
            .ok_or("buffer-derived fragment limit should be non-zero")?;
        FragmentationConfig::for_frame_budget(clamped, fragment_limit, Duration::from_secs(30))
            .ok_or_else(|| "failed to derive fragmentation config for test fixture".into())
    }

    /// Build the handler and payload channel shared by both startup modes.
    fn build_handler() -> (Handler<Envelope>, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let handler: Handler<Envelope> = std::sync::Arc::new(move |envelope: &Envelope| {
            let tx = tx.clone();
            let payload = envelope.payload_bytes().to_vec();
            Box::pin(async move {
                let _ = tx.send(payload);
            })
        });
        (handler, rx)
    }

    /// Start the app under test with derived budgets (no explicit
    /// `.memory_budgets(...)` call).
    pub fn start_app_derived(&mut self, buffer_capacity: usize) -> TestResult {
        let partial = WireframeApp::new()?.buffer_capacity(buffer_capacity);
        let fragmentation = Self::fragmentation_from_app(&partial)?;
        let (handler, rx) = Self::build_handler();

        let app = partial
            .fragmentation(Some(fragmentation))
            .with_message_assembler(TestAssembler)
            .route(ROUTE_ID, handler)?;

        self.start_with_app(app, rx)
    }

    /// Start the app under test with explicit budgets (overriding derived
    /// defaults).
    pub fn start_app_explicit(
        &mut self,
        buffer_capacity: usize,
        config: ExplicitBudgetConfig,
    ) -> TestResult {
        let partial = WireframeApp::new()?.buffer_capacity(buffer_capacity);
        let fragmentation = Self::fragmentation_from_app(&partial)?;

        let per_message =
            NonZeroUsize::new(config.per_message).ok_or("per_message must be non-zero")?;
        let per_connection =
            NonZeroUsize::new(config.per_connection).ok_or("per_connection must be non-zero")?;
        let in_flight = NonZeroUsize::new(config.in_flight).ok_or("in_flight must be non-zero")?;
        let budgets = MemoryBudgets::new(
            BudgetBytes::new(per_message),
            BudgetBytes::new(per_connection),
            BudgetBytes::new(in_flight),
        );

        let (handler, rx) = Self::build_handler();

        let app = partial
            .fragmentation(Some(fragmentation))
            .with_message_assembler(TestAssembler)
            .memory_budgets(budgets)
            .route(ROUTE_ID, handler)?;

        self.start_with_app(app, rx)
    }

    fn start_with_app(
        &mut self,
        app: WireframeApp,
        rx: mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> TestResult {
        let codec = app.length_codec();
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let client = Framed::new(client_stream, codec);

        self.block_on(async { tokio::time::pause() })?;
        let server = self
            .runtime()?
            .spawn(async move { app.handle_connection_result(server_stream).await });

        self.client = Some(client);
        self.server = Some(server);
        self.observed_rx = Some(rx);
        self.observed_payloads.clear();
        self.last_send_error = None;
        self.connection_error = None;
        Ok(())
    }

    /// Send multiple non-final first frames for keys in a range.
    pub fn send_first_frames_for_range(&mut self, start: u64, end: u64, body: &str) -> TestResult {
        for key in start..=end {
            if self.send_first_frame(key, body).is_err() {
                break;
            }
            self.spin_runtime()?;
        }
        Ok(())
    }

    /// Send a non-final first frame for the provided message key.
    pub fn send_first_frame(&mut self, key: u64, body: &str) -> TestResult {
        let payload = test_helpers::first_frame_payload(key, body.as_bytes(), false, None)?;
        self.send_payload(payload)
    }

    /// Send a final continuation frame for the provided message key.
    pub fn send_final_continuation_frame(
        &mut self,
        key: u64,
        sequence: u32,
        body: &str,
    ) -> TestResult {
        let payload =
            test_helpers::continuation_frame_payload(key, sequence, body.as_bytes(), true)?;
        self.send_payload(payload)
    }

    /// Assert that the connection has terminated with an error.
    pub fn assert_connection_aborted(&mut self) -> TestResult {
        self.spin_runtime()?;
        self.drain_ready_payloads()?;
        let server = self.server.take().ok_or("server not initialized")?;
        let result = self.block_on(server)?;
        match result {
            Ok(Ok(())) => Err("expected connection to abort, but it completed successfully".into()),
            Ok(Err(error)) => {
                self.connection_error = Some(error.to_string());
                Ok(())
            }
            Err(join_error) => Err(format!("server task panicked: {join_error}").into()),
        }
    }

    /// Assert that the expected payload is eventually observed.
    pub fn assert_payload_received(&mut self, expected: &str) -> TestResult {
        let expected = expected.as_bytes();
        for _ in 0..SPIN_ATTEMPTS {
            self.drain_ready_payloads()?;
            if self
                .observed_payloads
                .iter()
                .any(|payload| payload.as_slice() == expected)
            {
                return Ok(());
            }
            self.block_on(async { tokio::task::yield_now().await })?;
        }

        Err(format!(
            "expected payload {:?} not observed; observed={:?}",
            expected, self.observed_payloads
        )
        .into())
    }

    /// Assert that no connection error has occurred.
    ///
    /// Checks the server task directly rather than relying solely on the
    /// cached `connection_error` field, which is only populated by
    /// `assert_connection_aborted`. This prevents false negatives when the
    /// server errors after processing frames in non-abort scenarios.
    pub fn assert_no_connection_error(&mut self) -> TestResult {
        if let Some(ref error) = self.connection_error {
            return Err(format!("unexpected connection error: {error}").into());
        }
        // Drop the client so the server sees EOF and can finish cleanly.
        self.client.take();
        let server = self.server.take().ok_or("server not initialized")?;
        let result = self.block_on(server)?;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(format!("server task returned error: {error}").into()),
            Err(join_error) => Err(format!("server task panicked: {join_error}").into()),
        }
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
            Ok(Err(error)) => {
                self.last_send_error = Some(error.to_string());
                Err(error.into())
            }
            Err(error) => {
                self.last_send_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn spin_runtime(&self) -> TestResult {
        self.block_on(async {
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        })?;
        Ok(())
    }

    fn drain_ready_payloads(&mut self) -> TestResult {
        let mut observed_rx = self.observed_rx.take().ok_or("receiver not initialized")?;
        while let Ok(payload) = observed_rx.try_recv() {
            self.observed_payloads.push(payload);
        }
        self.observed_rx = Some(observed_rx);
        Ok(())
    }
}
