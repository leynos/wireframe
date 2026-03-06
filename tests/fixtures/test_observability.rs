//! Test world for observability harness behavioural scenarios.
//!
//! Exercises the `wireframe_testing` observability API to verify that
//! metrics and logs are captured and asserted correctly.

use rstest::fixture;
use wireframe_testing::ObservabilityHandle;
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

/// BDD world holding the observability handle and test state.
///
/// `ObservabilityHandle` contains a `LoggerHandle` that wraps a
/// `MutexGuard`, so it does not derive `Debug`.
#[derive(Default)]
pub struct TestObservabilityWorld {
    obs: Option<ObservabilityHandle>,
}

impl std::fmt::Debug for TestObservabilityWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestObservabilityWorld")
            .field("obs", &self.obs.as_ref().map(|_| ".."))
            .finish()
    }
}

/// Fixture for test observability scenarios used by rstest-bdd steps.
#[rustfmt::skip]
#[fixture]
pub fn test_observability_world() -> TestObservabilityWorld {
    TestObservabilityWorld::default()
}

impl TestObservabilityWorld {
    /// Acquire a fresh observability harness.
    ///
    /// # Errors
    ///
    /// Returns an error if the harness is already acquired.
    pub fn acquire_harness(&mut self) -> TestResult {
        if self.obs.is_some() {
            return Err("harness already acquired".into());
        }
        let mut obs = ObservabilityHandle::new();
        obs.clear();
        self.obs = Some(obs);
        Ok(())
    }

    /// Record a codec error metric via the local recorder.
    ///
    /// # Errors
    ///
    /// Returns an error if the harness has not been acquired.
    pub fn record_codec_error(&mut self) -> TestResult {
        let obs = self.obs.as_ref().ok_or("harness not acquired")?;
        metrics::with_local_recorder(obs.recorder(), || {
            wireframe::metrics::inc_codec_error("framing", "drop");
        });
        Ok(())
    }

    /// Emit a warning log message.
    ///
    /// # Errors
    ///
    /// Returns an error if the harness has not been acquired.
    pub fn emit_warning_log(&self) -> TestResult {
        let _obs = self.obs.as_ref().ok_or("harness not acquired")?;
        log::warn!("observability bdd test warning");
        Ok(())
    }

    /// Clear all captured observability state.
    ///
    /// # Errors
    ///
    /// Returns an error if the harness has not been acquired.
    pub fn clear_state(&mut self) -> TestResult {
        let obs = self.obs.as_mut().ok_or("harness not acquired")?;
        obs.clear();
        Ok(())
    }

    /// Verify the codec error counter matches the expected value.
    ///
    /// # Errors
    ///
    /// Returns an error if the counter does not match or the harness
    /// has not been acquired.
    pub fn verify_codec_error_counter(&mut self, expected: u64) -> TestResult {
        let obs = self.obs.as_mut().ok_or("harness not acquired")?;
        obs.snapshot();
        obs.assert_codec_error_counter("framing", "drop", expected)
            .map_err(Into::into)
    }

    /// Verify the log buffer contains the expected warning message.
    ///
    /// # Errors
    ///
    /// Returns an error if the expected message is not found or the
    /// harness has not been acquired.
    pub fn verify_log_contains(&mut self) -> TestResult {
        let obs = self.obs.as_mut().ok_or("harness not acquired")?;
        obs.assert_log_contains("observability bdd test warning")
            .map_err(Into::into)
    }
}
