//! Tests for push queue policy behaviour.

use std::sync::{Mutex, OnceLock};

use logtest::Logger;
use rstest::{fixture, rstest};
use wireframe::push::{PushPolicy, PushPriority, PushQueues};

/// Handle to the global logger with exclusive access.
struct LoggerHandle {
    guard: std::sync::MutexGuard<'static, Logger>,
}

impl LoggerHandle {
    fn new() -> Self {
        static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

        let logger = LOGGER.get_or_init(|| Mutex::new(Logger::start()));
        let mut guard = logger.lock().expect("lock logger");
        while guard.pop().is_some() {}

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

#[allow(unused_braces)]
#[fixture]
fn logger() -> LoggerHandle { LoggerHandle::new() }

#[rstest]
#[tokio::test]
async fn drop_if_full_discards_frame(mut logger: LoggerHandle) {
    let (mut queues, handle) = PushQueues::bounded(1, 1);
    handle.push_high_priority(1u8).await.unwrap();
    handle
        .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
        .unwrap();
    let (_, val) = queues.recv().await.unwrap();
    assert_eq!(val, 1);
    assert!(queues.high_priority_rx.try_recv().is_err());

    assert!(logger.pop().is_none());
}

#[rstest]
#[tokio::test]
async fn warn_and_drop_if_full_logs_warning(mut logger: LoggerHandle) {
    let (mut queues, handle) = PushQueues::bounded(1, 1);
    handle.push_low_priority(3u8).await.unwrap();
    handle
        .try_push(4u8, PushPriority::Low, PushPolicy::WarnAndDropIfFull)
        .unwrap();
    let (_, val) = queues.recv().await.unwrap();
    assert_eq!(val, 3);
    assert!(queues.low_priority_rx.try_recv().is_err());

    let record = logger.pop().expect("expected warning");
    assert_eq!(record.level(), log::Level::Warn);
    assert!(record.args().contains("push queue full"));
    assert!(logger.pop().is_none());
}
