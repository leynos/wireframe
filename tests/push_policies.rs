//! Tests for push queue policy behaviour.

use std::sync::{Mutex, OnceLock};

use logtest::Logger;
use rstest::{fixture, rstest};
use tokio::{
    runtime::Runtime,
    time::{Duration, timeout},
};
use wireframe::push::{PushPolicy, PushPriority, PushQueues};

/// Handle to the global logger with exclusive access.
struct LoggerHandle {
    guard: std::sync::MutexGuard<'static, Logger>,
}

impl LoggerHandle {
    fn new() -> Self {
        static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

        let logger = LOGGER.get_or_init(|| Mutex::new(Logger::start()));
        let guard = logger
            .lock()
            .expect("failed to acquire global logger lock; a previous test may still hold it");

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

#[allow(unused_braces)]
#[fixture]
fn rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build test runtime")
}

#[rstest]
fn drop_if_full_discards_frame(rt: Runtime, mut logger: LoggerHandle) {
    rt.block_on(async {
        let (mut queues, handle) = PushQueues::bounded(1, 1);
        handle.push_high_priority(1u8).await.unwrap();
        handle
            .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
            .unwrap();
        let (_, val) = queues.recv().await.unwrap();
        assert_eq!(val, 1);
        assert!(
            timeout(Duration::from_millis(20), queues.recv())
                .await
                .is_err()
        );

        assert!(logger.pop().is_none());
    });
}

#[rstest]
fn warn_and_drop_if_full_logs_warning(rt: Runtime, mut logger: LoggerHandle) {
    rt.block_on(async {
        let (mut queues, handle) = PushQueues::bounded(1, 1);
        handle.push_low_priority(3u8).await.unwrap();
        handle
            .try_push(4u8, PushPriority::Low, PushPolicy::WarnAndDropIfFull)
            .unwrap();
        let (_, val) = queues.recv().await.unwrap();
        assert_eq!(val, 3);
        assert!(
            timeout(Duration::from_millis(20), queues.recv())
                .await
                .is_err()
        );

        let record = logger.pop().expect("expected warning");
        assert_eq!(record.level(), log::Level::Warn);
        assert!(record.args().contains("push queue full"));
        assert!(logger.pop().is_none());
    });
}
