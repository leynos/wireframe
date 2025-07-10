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
    /// Acquire the global [`Logger`] instance.
    pub fn new() -> Self {
        static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

        let logger = LOGGER.get_or_init(|| Mutex::new(Logger::start()));
        let guard = logger.lock().expect("logger poisoned");

        Self { guard }
    }
}

impl std::ops::Deref for LoggerHandle {
    type Target = Logger;

    fn deref(&self) -> &Self::Target { &self.guard }
}

impl std::ops::DerefMut for LoggerHandle {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.guard }
}

#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
pub fn logger() -> LoggerHandle { LoggerHandle::new() }
