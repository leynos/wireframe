//! `MemoryBudgetsWorld` fixture for rstest-bdd tests.

use std::{num::NonZeroUsize, str::FromStr};

use rstest::fixture;
use wireframe::{
    app::{BudgetBytes as AppBudgetBytes, MemoryBudgets},
    codec::LengthDelimitedFrameCodec,
};
pub use wireframe_testing::TestResult;

use crate::TestApp;

/// Byte-count wrapper used by Gherkin steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepBudgetBytes(pub usize);

impl FromStr for StepBudgetBytes {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(Self) }
}

/// Behavioural test world for memory budget configuration.
#[derive(Debug, Default)]
pub struct MemoryBudgetsWorld {
    budgets: Option<MemoryBudgets>,
    budget_setup_error: Option<String>,
    app_configured: bool,
}

/// Fixture for `MemoryBudgetsWorld`.
#[fixture]
#[rustfmt::skip]
pub fn memory_budgets_world() -> MemoryBudgetsWorld {
    MemoryBudgetsWorld::default()
}

impl MemoryBudgetsWorld {
    /// Configure memory budgets in the world.
    ///
    /// # Errors
    ///
    /// Returns an error when any value is zero.
    pub fn set_budgets(
        &mut self,
        per_message: StepBudgetBytes,
        per_connection: StepBudgetBytes,
        in_flight: StepBudgetBytes,
    ) -> TestResult {
        let per_message =
            NonZeroUsize::new(per_message.0).ok_or("message budget must be non-zero")?;
        let per_connection =
            NonZeroUsize::new(per_connection.0).ok_or("connection budget must be non-zero")?;
        let in_flight =
            NonZeroUsize::new(in_flight.0).ok_or("in-flight budget must be non-zero")?;
        self.budgets = Some(MemoryBudgets::new(
            AppBudgetBytes::new(per_message),
            AppBudgetBytes::new(per_connection),
            AppBudgetBytes::new(in_flight),
        ));
        Ok(())
    }

    /// Attempt to configure memory budgets and store any resulting error.
    pub fn attempt_set_budgets(
        &mut self,
        per_message: StepBudgetBytes,
        per_connection: StepBudgetBytes,
        in_flight: StepBudgetBytes,
    ) {
        self.budget_setup_error = self
            .set_budgets(per_message, per_connection, in_flight)
            .err()
            .map(|error| error.to_string());
    }

    /// Build a `WireframeApp` configured with memory budgets.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or app construction fails.
    pub fn configure_app_with_budgets(&mut self) -> TestResult {
        let budgets = self.budgets.ok_or("memory budgets not configured")?;
        let _app = TestApp::new()
            .map_err(|err| format!("failed to build app: {err}"))?
            .memory_budgets(budgets)
            .read_timeout_ms(250);
        self.app_configured = true;
        Ok(())
    }

    /// Build a `WireframeApp` configured with memory budgets and a custom
    /// codec frame budget.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or app construction fails.
    pub fn configure_app_with_budgets_and_codec(&mut self) -> TestResult {
        let budgets = self.budgets.ok_or("memory budgets not configured")?;
        let _app = TestApp::new()
            .map_err(|err| format!("failed to build app: {err}"))?
            .memory_budgets(budgets)
            .with_codec(LengthDelimitedFrameCodec::new(2048));
        self.app_configured = true;
        Ok(())
    }

    /// Assert memory-budget configuration fails with the expected message.
    ///
    /// # Errors
    ///
    /// Returns an error if no setup failure was captured or if the message does
    /// not match.
    pub fn assert_budget_setup_failed_with(&self, expected: &str) -> TestResult {
        let actual = self
            .budget_setup_error
            .as_deref()
            .ok_or("expected memory-budget configuration to fail")?;
        if actual != expected {
            return Err(format!("expected setup error '{expected}', got '{actual}'").into());
        }
        Ok(())
    }

    /// Assert the message-level budget.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or value mismatches.
    pub fn assert_message_budget(&self, expected: StepBudgetBytes) -> TestResult {
        self.assert_budget("message", expected, |budgets| {
            budgets.bytes_per_message().as_usize()
        })
    }

    /// Assert the connection-level budget.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or value mismatches.
    pub fn assert_connection_budget(&self, expected: StepBudgetBytes) -> TestResult {
        self.assert_budget("connection", expected, |budgets| {
            budgets.bytes_per_connection().as_usize()
        })
    }

    /// Assert the in-flight budget.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or value mismatches.
    pub fn assert_in_flight_budget(&self, expected: StepBudgetBytes) -> TestResult {
        self.assert_budget("in-flight", expected, |budgets| {
            budgets.bytes_in_flight().as_usize()
        })
    }

    fn assert_budget<Accessor>(
        &self,
        budget_name: &str,
        expected: StepBudgetBytes,
        accessor: Accessor,
    ) -> TestResult
    where
        Accessor: Fn(&MemoryBudgets) -> usize,
    {
        let budgets = self.budgets.ok_or("memory budgets not configured")?;
        let actual = accessor(&budgets);
        if actual != expected.0 {
            return Err(format!(
                "expected {budget_name} budget {}, got {}",
                expected.0, actual
            )
            .into());
        }
        Ok(())
    }

    /// Assert app configuration completed successfully.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration has not completed.
    pub fn assert_configuration_succeeded(&self) -> TestResult {
        if self.app_configured {
            Ok(())
        } else {
            Err("expected app configuration to succeed".into())
        }
    }
}
