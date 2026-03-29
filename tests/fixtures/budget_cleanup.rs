//! Behavioural fixture for budget cleanup and reclamation scenarios (8.3.6).

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

use super::budget_config_parser::parse_standard_budget_config;

const ROUTE_ID: u32 = 92;
const CORRELATION_ID: Option<u64> = Some(21);
const BUFFER_CAPACITY: usize = 512;
const SPIN_ATTEMPTS: usize = 64;
const SERVER_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Parsed as
/// "`timeout_ms` / `per_message` / `per_connection` / `in_flight`".
#[derive(Clone, Copy, Debug)]
pub struct CleanupConfig {
    pub timeout_ms: u64,
    pub per_message: usize,
    pub per_connection: usize,
    pub in_flight: usize,
}

impl FromStr for CleanupConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = parse_standard_budget_config(s)?;
        Ok(Self {
            timeout_ms: parsed.timeout_ms,
            per_message: parsed.per_message,
            per_connection: parsed.per_connection,
            in_flight: parsed.in_flight,
        })
    }
}

/// Runtime-backed fixture that drives inbound assembly with memory budgets
/// and validates cleanup and budget reclamation semantics.
pub struct BudgetCleanupWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    client: Option<Framed<DuplexStream, LengthDelimitedCodec>>,
    server: Option<JoinHandle<std::io::Result<()>>>,
    observed_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    observed_payloads: Vec<Vec<u8>>,
    last_send_error: Option<String>,
    connection_error: Option<String>,
}

impl BudgetCleanupWorld {
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

impl Default for BudgetCleanupWorld {
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

impl fmt::Debug for BudgetCleanupWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BudgetCleanupWorld")
            .field("client_initialized", &self.client.is_some())
            .field("server_initialized", &self.server.is_some())
            .field("observed_payloads", &self.observed_payloads.len())
            .field("last_send_error", &self.last_send_error)
            .field("connection_error", &self.connection_error)
            .finish_non_exhaustive()
    }
}

/// Construct the default world used by budget cleanup BDD tests.
#[rustfmt::skip]
#[fixture]
pub fn budget_cleanup_world() -> BudgetCleanupWorld {
    BudgetCleanupWorld::default()
}

impl BudgetCleanupWorld {
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
            return Err("nested Tokio runtime detected in cleanup fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    /// Start the app under test using the supplied budget and timeout config.
    pub fn start_app(&mut self, config: CleanupConfig) -> TestResult {
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

    /// Advance Tokio virtual time by the supplied duration in milliseconds.
    pub fn advance_millis(&mut self, millis: u64) -> TestResult {
        self.block_on(async { tokio::time::advance(Duration::from_millis(millis)).await })?;
        Ok(())
    }

    /// Drop the client half of the duplex stream, triggering EOF on the
    /// server side. This exercises the RAII cleanup path where
    /// `MessageAssemblyState` (and all partial assemblies) is freed by the
    /// normal scope-based drop in `process_stream`.
    pub fn disconnect_client(&mut self) { self.client.take(); }

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
    /// Drops the client (if still held) so the server sees EOF and can
    /// finish cleanly, then joins the server task with a bounded timeout.
    pub fn assert_no_connection_error(&mut self) -> TestResult {
        if let Some(ref error) = self.connection_error {
            return Err(format!("unexpected connection error: {error}").into());
        }
        // Drop the client so the server sees EOF and can finish cleanly.
        self.client.take();
        match self.join_server()? {
            Ok(()) => Ok(()),
            Err(error) => Err(format!("server task returned error: {error}").into()),
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

        self.record_send_result(send_result)
    }

    fn record_send_result(&mut self, send_result: TestResult<std::io::Result<()>>) -> TestResult {
        let result = send_result.and_then(|io_result| io_result.map_err(Into::into));
        if let Err(ref error) = result {
            self.last_send_error = Some(error.to_string());
        } else {
            self.last_send_error = None;
        }
        result
    }

    /// Join the server task with a bounded timeout to prevent indefinite
    /// hangs.
    fn join_server(&mut self) -> TestResult<std::io::Result<()>> {
        let server = self.server.take().ok_or("server not initialized")?;
        let join_result =
            self.block_on(async { tokio::time::timeout(SERVER_JOIN_TIMEOUT, server).await })?;
        match join_result {
            Ok(Ok(io_result)) => Ok(io_result),
            Ok(Err(join_error)) => Err(format!("server task panicked: {join_error}").into()),
            Err(_elapsed) => Err("server task did not complete within timeout".into()),
        }
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

#[cfg(test)]
mod tests {
    //! Fixture self-tests for budget cleanup world wiring and parsing.

    use std::io;

    use super::BudgetCleanupWorld;

    #[test]
    fn record_send_result_clears_last_error_after_success() {
        let mut world = BudgetCleanupWorld::default();
        let first_failure = io::Error::other("first failure");
        let first_result = world.record_send_result(Ok(Err(first_failure)));
        let first_error = first_result.expect_err("first send should fail");
        assert_eq!(first_error.to_string(), "io error: first failure");
        assert_eq!(
            world.last_send_error.as_deref(),
            Some("io error: first failure")
        );

        let success_result = world.record_send_result(Ok(Ok(())));
        assert!(success_result.is_ok(), "second send should succeed");
        assert_eq!(world.last_send_error, None);
    }

    #[test]
    fn record_send_result_stores_outer_error_string() {
        let mut world = BudgetCleanupWorld::default();
        let outer_error = io::Error::other("outer failure");

        let result = world.record_send_result(Err(outer_error.into()));
        let returned_error = result.expect_err("outer error should be returned");
        assert_eq!(returned_error.to_string(), "io error: outer failure");
        assert_eq!(
            world.last_send_error.as_deref(),
            Some("io error: outer failure")
        );
    }

    #[test]
    fn record_send_result_converts_inner_io_error_and_stores_it() {
        let mut world = BudgetCleanupWorld::default();
        let result = world.record_send_result(Ok(Err(io::Error::other("inner io failure"))));
        let returned_error = result.expect_err("inner IO error should be returned");
        assert_eq!(returned_error.to_string(), "io error: inner io failure");
        assert_eq!(
            world.last_send_error.as_deref(),
            Some("io error: inner io failure")
        );
    }
}
