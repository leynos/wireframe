//! `MemoryBudgetsWorld` fixture for rstest-bdd tests.

use std::{num::NonZeroUsize, str::FromStr};

use rstest::fixture;
use wireframe::{app::MemoryBudgets, codec::LengthDelimitedFrameCodec};
pub use wireframe_testing::TestResult;

use crate::TestApp;

/// Byte-count wrapper used by Gherkin steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetBytes(pub usize);

impl FromStr for BudgetBytes {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(Self) }
}

/// Behavioural test world for memory budget configuration.
#[derive(Debug, Default)]
pub struct MemoryBudgetsWorld {
    budgets: Option<MemoryBudgets>,
    app_configured: bool,
}

/// Fixture for `MemoryBudgetsWorld`.
#[rustfmt::skip]
#[fixture]
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
        per_message: BudgetBytes,
        per_connection: BudgetBytes,
        in_flight: BudgetBytes,
    ) -> TestResult {
        let per_message =
            NonZeroUsize::new(per_message.0).ok_or("message budget must be non-zero")?;
        let per_connection =
            NonZeroUsize::new(per_connection.0).ok_or("connection budget must be non-zero")?;
        let in_flight =
            NonZeroUsize::new(in_flight.0).ok_or("in-flight budget must be non-zero")?;
        self.budgets = Some(MemoryBudgets::new(per_message, per_connection, in_flight));
        Ok(())
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

    /// Assert the message-level budget.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or value mismatches.
    pub fn assert_message_budget(&self, expected: BudgetBytes) -> TestResult {
        let budgets = self.budgets.ok_or("memory budgets not configured")?;
        if budgets.bytes_per_message().get() != expected.0 {
            return Err(format!(
                "expected message budget {}, got {}",
                expected.0,
                budgets.bytes_per_message().get()
            )
            .into());
        }
        Ok(())
    }

    /// Assert the connection-level budget.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or value mismatches.
    pub fn assert_connection_budget(&self, expected: BudgetBytes) -> TestResult {
        let budgets = self.budgets.ok_or("memory budgets not configured")?;
        if budgets.bytes_per_connection().get() != expected.0 {
            return Err(format!(
                "expected connection budget {}, got {}",
                expected.0,
                budgets.bytes_per_connection().get()
            )
            .into());
        }
        Ok(())
    }

    /// Assert the in-flight budget.
    ///
    /// # Errors
    ///
    /// Returns an error if budgets are missing or value mismatches.
    pub fn assert_in_flight_budget(&self, expected: BudgetBytes) -> TestResult {
        let budgets = self.budgets.ok_or("memory budgets not configured")?;
        if budgets.bytes_in_flight().get() != expected.0 {
            return Err(format!(
                "expected in-flight budget {}, got {}",
                expected.0,
                budgets.bytes_in_flight().get()
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
