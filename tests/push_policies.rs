//! Tests for push queue policy behaviour.

use std::sync::{Mutex, Once};

use logtest::Logger;
use rstest::{fixture, rstest};
use wireframe::push::{PushPolicy, PushPriority, PushQueues};

/// Handle to the global logger with exclusive access.
struct LoggerHandle {
    _guard: std::sync::MutexGuard<'static, ()>,
    logger: Logger,
}

impl LoggerHandle {
    fn new() -> Self {
        static INIT: Once = Once::new();
        static LOCK: Mutex<()> = Mutex::new(());

        let guard = LOCK.lock().expect("lock logger");
        INIT.call_once(|| {
            Logger::start();
        });

        let mut logger = Logger;
        while logger.pop().is_some() {}

        Self {
            _guard: guard,
            logger,
        }
    }
}

impl std::ops::Deref for LoggerHandle {
    type Target = Logger;

    fn deref(&self) -> &Self::Target { &self.logger }
}

impl std::ops::DerefMut for LoggerHandle {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.logger }
}

#[fixture]
fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}
#[allow(unused_braces)]
#[fixture]
fn logger() -> LoggerHandle { LoggerHandle::new() }

#[rstest]
fn drop_if_full_discards_frame(rt: tokio::runtime::Runtime, mut logger: LoggerHandle) {
    rt.block_on(async {
        let (mut queues, handle) = PushQueues::bounded(1, 1);
        handle.push_high_priority(1u8).await.unwrap();
        handle
            .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
            .unwrap();
        let (_, val) = queues.recv().await.unwrap();
        assert_eq!(val, 1);
        assert!(queues.high_priority_rx.try_recv().is_err());
    });

    assert!(logger.pop().is_none());
}

#[rstest]
fn warn_and_drop_if_full_logs_warning(rt: tokio::runtime::Runtime, mut logger: LoggerHandle) {
    rt.block_on(async {
        let (mut queues, handle) = PushQueues::bounded(1, 1);
        handle.push_low_priority(3u8).await.unwrap();
        handle
            .try_push(4u8, PushPriority::Low, PushPolicy::WarnAndDropIfFull)
            .unwrap();
        let (_, val) = queues.recv().await.unwrap();
        assert_eq!(val, 3);
        assert!(queues.low_priority_rx.try_recv().is_err());
    });

    let record = logger.pop().expect("expected warning");
    assert_eq!(record.level(), log::Level::Warn);
    assert!(record.args().contains("push queue full"));
    assert!(logger.pop().is_none());
}
