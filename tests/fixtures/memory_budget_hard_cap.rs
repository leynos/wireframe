//! Behavioural fixture for hard-cap memory budget connection abort scenarios.

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

const ROUTE_ID: u32 = 89;
const CORRELATION_ID: Option<u64> = Some(10);
const BUFFER_CAPACITY: usize = 512;
const SPIN_ATTEMPTS: usize = 64;

/// Parsed as
/// "`timeout_ms` / `per_message` / `per_connection` / `in_flight`".
#[derive(Clone, Copy, Debug)]
pub struct HardCapConfig {
    pub timeout_ms: u64,
    pub per_message: usize,
    pub per_connection: usize,
    pub in_flight: usize,
}

impl FromStr for HardCapConfig {
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

/// Runtime-backed fixture that drives inbound assembly with memory budgets
/// and validates connection termination behaviour.
pub struct MemoryBudgetHardCapWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    client: Option<Framed<DuplexStream, LengthDelimitedCodec>>,
    server: Option<JoinHandle<std::io::Result<()>>>,
    observed_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    observed_payloads: Vec<Vec<u8>>,
    last_send_error: Option<String>,
    connection_error: Option<String>,
}

impl MemoryBudgetHardCapWorld {
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

impl Default for MemoryBudgetHardCapWorld {
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

impl fmt::Debug for MemoryBudgetHardCapWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryBudgetHardCapWorld")
            .field("client_initialized", &self.client.is_some())
            .field("server_initialized", &self.server.is_some())
            .field("observed_payloads", &self.observed_payloads.len())
            .field("last_send_error", &self.last_send_error)
            .field("connection_error", &self.connection_error)
            .finish_non_exhaustive()
    }
}

/// Construct the default world used by hard-cap memory budget BDD tests.
#[fixture]
pub fn memory_budget_hard_cap_world() -> MemoryBudgetHardCapWorld {
    MemoryBudgetHardCapWorld::default()
}

impl MemoryBudgetHardCapWorld {
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
            return Err("nested Tokio runtime detected in hard-cap fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    /// Start the app under test using the supplied budget and timeout config.
    pub fn start_app(&mut self, config: HardCapConfig) -> TestResult {
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
        self.connection_error = None;
        Ok(())
    }

    /// Send multiple non-final first frames for keys in a range.
    pub fn send_first_frames_for_range(&mut self, start: u64, end: u64, body: &str) -> TestResult {
        for key in start..=end {
            // Ignore send errors — the connection may close mid-way through
            // and we want to observe the connection error, not the send error.
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

    /// Assert that the connection has terminated with an error and that no
    /// payloads were delivered before the abort.
    pub fn assert_connection_aborted(&mut self) -> TestResult {
        self.spin_runtime()?;
        self.drain_ready_payloads()?;
        if !self.observed_payloads.is_empty() {
            return Err(format!(
                "expected no payloads before abort, but received: {:?}",
                self.observed_payloads
            )
            .into());
        }
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

    /// Assert that no connection error has been recorded.
    pub fn assert_no_connection_error(&self) -> TestResult {
        if self.connection_error.is_none() {
            return Ok(());
        }
        Err(format!("unexpected connection error: {:?}", self.connection_error).into())
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

#[cfg(test)]
mod hard_cap_config_tests {
    use std::str::FromStr;

    use rstest::rstest;

    use super::HardCapConfig;

    #[rstest]
    fn valid_four_segment_input() {
        let c = HardCapConfig::from_str("200/2048/8/8").expect("valid input");
        assert_eq!(
            (c.timeout_ms, c.per_message, c.per_connection, c.in_flight),
            (200, 2048, 8, 8)
        );
    }

    #[rstest]
    #[case::missing_first("/64/100/100")]
    #[case::missing_second("10//100/100")]
    #[case::missing_third("10/64//100")]
    #[case::missing_fourth("10/64/100/")]
    #[case::only_three("10/64/100")]
    #[case::extra_segment("10/64/100/100/extra")]
    #[case::alphabetic_timeout("abc/64/100/100")]
    #[case::alphabetic_budget("10/abc/100/100")]
    #[case::negative_value("10/64/-1/100")]
    fn invalid_inputs_are_rejected(#[case] input: &str) {
        assert!(
            HardCapConfig::from_str(input).is_err(),
            "expected Err for {input:?}"
        );
    }

    #[rstest]
    fn zero_timeout_is_accepted() {
        let c = HardCapConfig::from_str("0/64/100/100").expect("zero timeout is valid");
        assert_eq!(c.timeout_ms, 0);
    }

    // Zero budgets parse as valid `usize` values. The non-zero constraint is
    // enforced later by `start_app()` via `NonZeroUsize::new(...)`.
    #[rstest]
    fn zero_budget_parses_successfully() {
        let c = HardCapConfig::from_str("10/0/100/100").expect("zero budget parses");
        assert_eq!(c.per_message, 0);
    }
}
