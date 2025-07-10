//! Logging utilities for test infrastructure.
//!
//! This module provides a global, thread-safe logger handle for capturing and
//! inspecting log output during tests. The [`LoggerHandle`] ensures exclusive
//! access to prevent interference between concurrent tests.

use std::sync::{Mutex, MutexGuard, OnceLock};

use logtest::Logger;
use rstest::fixture;
/// Handle to the global logger with exclusive access.
///
/// This guard ensures tests do not interfere with each other's log capture by
/// serialising access to a [`logtest::Logger`].
pub struct LoggerHandle {
    guard: MutexGuard<'static, Logger>,
}

impl LoggerHandle {
    /// Acquires exclusive access to the global logger instance for testing.
    ///
    /// Returns a handle that provides serialised, thread-safe access to a singleton
    /// [`Logger`], ensuring that concurrent tests do not interfere with each other's
    /// log capture. Panics if the logger mutex is poisoned.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let handle = LoggerHandle::new();
    /// handle.clear(); // Clear previous logs before running a test.
    /// ```
    pub fn new() -> Self {
        static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

        let logger = LOGGER.get_or_init(|| Mutex::new(Logger::start()));
        let guard = logger.lock().expect("logger poisoned");

        Self { guard }
    }
}

impl std::ops::Deref for LoggerHandle {
    type Target = Logger;

    /// Returns a reference to the underlying logger, enabling read-only access through the handle.
///
/// # Examples
///
/// ```no_run
/// let handle = LoggerHandle::new();
/// let logs = handle.logs();
/// ```
fn deref(&self) -> &Self::Target { &self.guard }
}

impl std::ops::DerefMut for LoggerHandle {
    /// Provides mutable access to the underlying `Logger` instance.
///
/// # Examples
///
/// ```no_run
/// use wireframe_testing::logging::LoggerHandle;
///
/// let mut handle = LoggerHandle::new();
/// handle.clear(); // Mutably access the logger to clear captured logs
/// ```
fn deref_mut(&mut self) -> &mut Self::Target { &mut self.guard }
}

/// Provides a test fixture that returns exclusive access to the global logger.
///
/// This fixture ensures that each test receives a unique handle to the global
/// logger, allowing safe capture and inspection of log output without
/// interference from other tests.
///
/// # Examples
///
/// ```no_run
/// use wireframe_testing::logging::logger;
///
/// #[rstest::rstest]
/// fn test_logging(logger: LoggerHandle) {
///     logger.clear();
///     // ... perform actions that produce logs ...
///     let logs = logger.pop();
///     assert!(logs.contains("expected log message"));
/// }
/// ```
#[allow(
unused_braces,
reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
pub fn logger() -> LoggerHandle { LoggerHandle::new() }
