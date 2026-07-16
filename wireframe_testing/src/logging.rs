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
/// serializing access to a [`logtest::Logger`]. Acquire it using [`logger`] or
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

/// Returns a static reference to the shared global logger [`Mutex`].
///
/// The logger is initialized on first access via a [`OnceLock`]. All
/// [`LoggerHandle`] instances share this single mutex, which serializes
/// log capture across concurrent tests.
///
/// If a prior test panicked while holding the mutex, callers can recover
/// the guard via [`std::sync::PoisonError::into_inner`] (see
/// [`LoggerHandle::new`]) without losing access to the logger.
fn shared_logger() -> &'static Mutex<Logger> {
    static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();
    LOGGER.get_or_init(|| Mutex::new(Logger::start()))
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
        // Preserve the shared logger even if a prior test panicked while
        // holding the mutex, but clear any buffered state so the next test
        // starts from a clean log view.
        let guard = match shared_logger().lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                while guard.pop().is_some() {}
                guard
            }
        };

        Self { guard }
    }

    /// Remove all currently buffered log records.
    pub fn clear(&mut self) { while self.pop().is_some() {} }
}

impl Default for LoggerHandle {
    fn default() -> Self { Self::new() }
}

impl std::ops::Deref for LoggerHandle {
    type Target = Logger;

    fn deref(&self) -> &Self::Target { &self.guard }
}

impl std::ops::DerefMut for LoggerHandle {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.guard }
}

/// rstest fixture returning a [`LoggerHandle`] for log assertions.
#[fixture]
pub fn logger() -> LoggerHandle {
    // Acquire exclusive access to the global logger for test assertions
    LoggerHandle::new()
}

#[cfg(test)]
mod tests {
    use super::{LoggerHandle, shared_logger};

    #[test]
    fn default_returns_usable_handle() {
        // Verify that Default delegates to new() and the handle is functional.
        let mut handle = LoggerHandle::default();
        // The handle must allow log draining without panicking.
        handle.clear();
    }

    #[test]
    fn default_poison_recovery_returns_usable_handle() {
        let mut handle = LoggerHandle::default();
        handle.clear();
        drop(handle);

        let join_result = std::thread::spawn(|| {
            let _guard = shared_logger()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            log::info!("stale");
            panic!("poison logger mutex");
        })
        .join();
        assert!(join_result.is_err(), "poisoning thread should panic");

        let mut handle = LoggerHandle::default();
        assert!(
            handle.pop().is_none(),
            "poison recovery should drain stale log records"
        );
        handle.clear();
    }
}
