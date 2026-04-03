//! Assertion helpers for derived-memory-budget fixture outcomes.

use wireframe_testing::TestResult;

use super::{DerivedMemoryBudgetsWorld, SPIN_ATTEMPTS};

impl DerivedMemoryBudgetsWorld {
    /// Assert that the connection has terminated with an error.
    pub fn assert_connection_aborted(&mut self) -> TestResult {
        self.spin_runtime()?;
        self.drain_ready_payloads()?;
        match self.join_server()? {
            Ok(()) => Err("expected connection to abort, but it completed successfully".into()),
            Err(error) => {
                self.connection_error = Some(error.to_string());
                Ok(())
            }
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
        match self.join_server()? {
            Ok(()) => Ok(()),
            Err(error) => Err(format!("server task returned error: {error}").into()),
        }
    }
}
