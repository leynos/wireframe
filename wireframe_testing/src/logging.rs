//! Logging utilities for test infrastructure.
//!
//! This module provides a global, thread-safe logger handle for capturing and
//! inspecting log output during tests. The [`LoggerHandle`] ensures exclusive
//! access to prevent interference between concurrent tests.
//!
//! ```
//! use wireframe_testing::logger;
//!
//! #[tokio::test]
//! async fn logs_are_collected() {
//!     let mut log = logger();
//!     log::info!("example");
//!     assert!(log.pop().is_some());
//! }
//! ```

use std::sync::{Mutex, MutexGuard, OnceLock};

use logtest::Logger;
use rstest::fixture;
/// Handle to the global logger with exclusive access.
///
/// This guard ensures tests do not interfere with each other's log capture by
/// serialising access to a [`logtest::Logger`]. Acquire it using [`logger`] or
/// [`LoggerHandle::new`].
///
/// ```
/// use wireframe_testing::logger;
/// # use log::warn;
///
/// let mut log = logger();
/// warn!("warned");
/// assert!(log.pop().is_some());
/// assert!(log.pop().is_none());
/// ```
pub struct LoggerHandle {
    guard: MutexGuard<'static, Logger>,
}

impl LoggerHandle {
    /// Acquire the global [`Logger`] instance.
    ///
    /// ```no_run
    /// use wireframe_testing::LoggerHandle;
    /// # use log::warn;
    ///
    /// let mut log = LoggerHandle::new();
    /// warn!("warned");
    /// assert!(log.pop().is_some());
    /// assert!(log.pop().is_none());
    /// ```
    pub fn new() -> Self {
        static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

        let logger = LOGGER.get_or_init(|| Mutex::new(Logger::start()));
        let guard = logger.lock().expect("logger poisoned");

        Self { guard }
    }

    /// Remove all currently buffered log records.
    pub fn clear(&mut self) { while self.pop().is_some() {} }
}

impl std::ops::Deref for LoggerHandle {
    type Target = Logger;

    fn deref(&self) -> &Self::Target { &self.guard }
}

impl std::ops::DerefMut for LoggerHandle {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.guard }
}

/// rstest fixture returning a [`LoggerHandle`] for log assertions.
#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
pub fn logger() -> LoggerHandle { LoggerHandle::new() }
