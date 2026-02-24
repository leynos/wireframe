//! Behavioural fixture for soft-limit memory-budget back-pressure scenarios.

use std::{fmt, future::Future, num::NonZeroUsize, str::FromStr, time::Duration};

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

const ROUTE_ID: u32 = 88;
const CORRELATION_ID: Option<u64> = Some(9);
const BUFFER_CAPACITY: usize = 512;
const SPIN_ATTEMPTS: usize = 64;

/// Parsed as
/// "`timeout_ms` / `per_message` / `per_connection` / `in_flight`".
#[derive(Clone, Copy, Debug)]
pub struct BackpressureConfig {
    pub timeout_ms: u64,
    pub per_message: usize,
    pub per_connection: usize,
    pub in_flight: usize,
}

impl FromStr for BackpressureConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut values = s.split('/').map(str::trim);
        let timeout_ms = values
            .next()
            .filter(|value| !value.is_empty())
            .ok_or("missing timeout_ms")?;
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
            timeout_ms: timeout_ms
                .parse()
                .map_err(|error| format!("timeout_ms: {error}"))?,
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

/// Runtime-backed fixture that drives inbound assembly with memory budgets.
pub struct MemoryBudgetBackpressureWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    client: Option<Framed<DuplexStream, LengthDelimitedCodec>>,
    server: Option<JoinHandle<std::io::Result<()>>>,
    observed_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    observed_payloads: Vec<Vec<u8>>,
    last_send_error: Option<String>,
}

impl MemoryBudgetBackpressureWorld {
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
        }
    }
}

impl Default for MemoryBudgetBackpressureWorld {
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

impl fmt::Debug for MemoryBudgetBackpressureWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryBudgetBackpressureWorld")
            .field("client_initialized", &self.client.is_some())
            .field("server_initialized", &self.server.is_some())
            .field("observed_payloads", &self.observed_payloads.len())
            .field("last_send_error", &self.last_send_error)
            .finish_non_exhaustive()
    }
}

/// Construct the default world used by memory-budget back-pressure BDD tests.
#[fixture]
pub fn memory_budget_backpressure_world() -> MemoryBudgetBackpressureWorld {
    MemoryBudgetBackpressureWorld::default()
}

impl MemoryBudgetBackpressureWorld {
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
            return Err("nested Tokio runtime detected in memory-budget fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    /// Start the app under test using the supplied budget and timeout config.
    pub fn start_app(&mut self, config: BackpressureConfig) -> TestResult {
        let Some(fragment_limit) = NonZeroUsize::new(BUFFER_CAPACITY.saturating_mul(16)) else {
            return Err("buffer-derived fragment limit should be non-zero".into());
        };
        let fragmentation = FragmentationConfig::for_frame_budget(
            BUFFER_CAPACITY,
            fragment_limit,
            Duration::from_millis(config.timeout_ms),
        )
        .ok_or("failed to derive fragmentation config for test fixture")?;

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

        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let handler: Handler<Envelope> = std::sync::Arc::new(move |envelope: &Envelope| {
            let tx = tx.clone();
            let payload = envelope.payload_bytes().to_vec();
            Box::pin(async move {
                let _ = tx.send(payload);
            })
        });

        let app: WireframeApp = WireframeApp::new()?
            .buffer_capacity(BUFFER_CAPACITY)
            .fragmentation(Some(fragmentation))
            .with_message_assembler(TestAssembler)
            .memory_budgets(budgets)
            .route(ROUTE_ID, handler)?;

        let codec = app.length_codec();
        let (client_stream, server_stream) = tokio::io::duplex(2048);
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

    /// Assert that no payload has been emitted yet.
    pub fn assert_no_payload_ready(&mut self) -> TestResult {
        self.spin_runtime()?;
        self.drain_ready_payloads()?;
        if self.observed_payloads.is_empty() {
            return Ok(());
        }
        Err(format!(
            "expected no payload to be ready, observed={:?}",
            self.observed_payloads
        )
        .into())
    }

    /// Advance Tokio virtual time by the supplied duration in milliseconds.
    pub fn advance_millis(&mut self, millis: u64) -> TestResult {
        self.block_on(async { tokio::time::advance(Duration::from_millis(millis)).await })?;
        Ok(())
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

    /// Assert that sending frames into the app did not record any send error.
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

        // Temporarily move the framed client into the async block so send/flush
        // can run without holding a mutable borrow of `self`; then restore it.
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
