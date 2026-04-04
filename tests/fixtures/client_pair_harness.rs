//! BDD world fixture for in-process server and client pair harness scenarios.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rstest::fixture;
use wireframe::message::Message;
pub use wireframe_testing::TestResult;
use wireframe_testing::{
    CommonTestEnvelope,
    WireframePair,
    echo_app_factory,
    spawn_wireframe_pair,
    spawn_wireframe_pair_default,
};

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct Echo(u8);

/// BDD world for client pair harness scenarios.
pub struct ClientPairHarnessWorld {
    runtime: tokio::runtime::Runtime,
    counter: Arc<AtomicUsize>,
    pair: Option<WireframePair>,
    response: Option<CommonTestEnvelope>,
}

impl std::fmt::Debug for ClientPairHarnessWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientPairHarnessWorld")
            .field("runtime", &"..")
            .field("counter", &self.counter.load(Ordering::SeqCst))
            .field("pair", &self.pair.as_ref().map(|_| ".."))
            .field("response", &self.response)
            .finish()
    }
}

impl Default for ClientPairHarnessWorld {
    #[expect(
        clippy::expect_used,
        reason = "BDD world cannot propagate errors from Default"
    )]
    fn default() -> Self {
        Self {
            runtime: tokio::runtime::Runtime::new().expect("failed to create runtime"),
            counter: Arc::new(AtomicUsize::new(0)),
            pair: None,
            response: None,
        }
    }
}

/// Fixture for `ClientPairHarnessWorld`.
#[rustfmt::skip]
#[fixture]
pub fn client_pair_harness_world() -> ClientPairHarnessWorld {
    ClientPairHarnessWorld::default()
}

impl ClientPairHarnessWorld {
    /// Start a pair with default client settings.
    ///
    /// # Errors
    ///
    /// Returns an error if spawning the pair fails.
    pub fn start_default_pair(&mut self) -> TestResult {
        let factory = echo_app_factory(&self.counter);
        let pair = self
            .runtime
            .block_on(spawn_wireframe_pair_default(factory))?;
        self.pair = Some(pair);
        Ok(())
    }

    /// Start a pair with a custom max frame length.
    ///
    /// # Errors
    ///
    /// Returns an error if spawning the pair fails.
    pub fn start_pair_with_max_frame_length(&mut self, max_frame_length: usize) -> TestResult {
        let factory = echo_app_factory(&self.counter);
        let pair = self
            .runtime
            .block_on(spawn_wireframe_pair(factory, |builder| {
                builder.max_frame_length(max_frame_length)
            }))?;
        self.pair = Some(pair);
        Ok(())
    }

    /// Send a request and store the response.
    ///
    /// # Errors
    ///
    /// Returns an error if the pair is not started or the call fails.
    pub fn send_request(&mut self, id: u32, correlation_id: u64) -> TestResult {
        let pair = self.pair.as_mut().ok_or("pair not started")?;
        let payload_bytes = Echo(42).to_bytes()?;
        let request = CommonTestEnvelope::new(id, Some(correlation_id), payload_bytes);
        let response: CommonTestEnvelope =
            self.runtime.block_on(pair.client_mut().call(&request))?;
        self.response = Some(response);
        Ok(())
    }

    /// Assert the response has the expected correlation ID and payload.
    ///
    /// # Errors
    ///
    /// Returns an error if the response does not match expectations.
    pub fn assert_response(&self, expected_correlation_id: u64) -> TestResult {
        let response = self.response.as_ref().ok_or("no response stored")?;
        if response.correlation_id != Some(expected_correlation_id) {
            return Err(format!(
                "expected correlation id {expected_correlation_id}, got {:?}",
                response.correlation_id,
            )
            .into());
        }
        let (echo, _) = Echo::from_bytes(&response.payload)?;
        if echo != Echo(42) {
            return Err(format!("expected Echo(42), got {echo:?}").into());
        }
        Ok(())
    }
}

impl Drop for ClientPairHarnessWorld {
    fn drop(&mut self) {
        if let Some(mut pair) = self.pair.take() {
            let _ = self.runtime.block_on(pair.shutdown());
        }
    }
}
