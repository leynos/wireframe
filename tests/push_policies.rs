//! Tests for push queue policy behaviour.

use std::sync::{Mutex, OnceLock};

use futures::future::BoxFuture;
use logtest::Logger;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::{
    runtime::Runtime,
    sync::mpsc,
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

#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
fn logger() -> LoggerHandle { LoggerHandle::new() }

#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
fn rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build test runtime")
}

#[rstest]
#[serial(push_policies)]
fn drop_if_full_discards_frame(rt: Runtime, mut logger: LoggerHandle) {
    rt.block_on(async {
        while logger.pop().is_some() {}
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

        while let Some(record) = logger.pop() {
            assert!(
                !record.args().contains("push queue full"),
                "unexpected log: {}",
                record.args()
            );
        }
    });
}

#[rstest]
#[serial(push_policies)]
fn warn_and_drop_if_full_logs_warning(rt: Runtime, mut logger: LoggerHandle) {
    rt.block_on(async {
        while logger.pop().is_some() {}
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
        let mut found = false;
        while let Some(record) = logger.pop() {
            if record.level() == log::Level::Warn && record.args().contains("push queue full") {
                found = true;
                break;
            }
        }
        assert!(found, "warning log not found");
    });
}

#[rstest]
fn dropped_frame_goes_to_dlq(rt: Runtime) {
    rt.block_on(async {
        let (dlq_tx, mut dlq_rx) = mpsc::channel(1);
        let (mut queues, handle) =
            PushQueues::bounded_with_rate_dlq(1, 1, None, Some(dlq_tx)).unwrap();

        handle.push_high_priority(1u8).await.unwrap();
        handle
            .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
            .unwrap();

        let (_, val) = queues.recv().await.unwrap();
        assert_eq!(val, 1);
        assert_eq!(dlq_rx.recv().await.unwrap(), 2);
    });
}

fn setup_dlq_full(tx: &mpsc::Sender<u8>, _rx: &mut Option<mpsc::Receiver<u8>>) {
    tx.try_send(99).unwrap();
}

fn setup_dlq_closed(_: &mpsc::Sender<u8>, rx: &mut Option<mpsc::Receiver<u8>>) { drop(rx.take()); }

fn assert_dlq_full(rx: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, ()> {
    Box::pin(async move {
        let receiver = rx.as_mut().expect("receiver missing");
        assert_eq!(receiver.recv().await.unwrap(), 99);
        assert!(receiver.try_recv().is_err());
    })
}

fn assert_dlq_closed(_: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, ()> { Box::pin(async {}) }

#[rstest]
#[case::dlq_full(setup_dlq_full, PushPolicy::WarnAndDropIfFull, "DLQ", assert_dlq_full)]
#[case::dlq_closed(setup_dlq_closed, PushPolicy::DropIfFull, "closed", assert_dlq_closed)]
#[serial(push_policies)]
fn dlq_error_scenarios<Setup, AssertFn>(
    rt: Runtime,
    mut logger: LoggerHandle,
    #[case] setup: Setup,
    #[case] policy: PushPolicy,
    #[case] expected: &str,
    #[case] assertion: AssertFn,
) where
    Setup: FnOnce(&mpsc::Sender<u8>, &mut Option<mpsc::Receiver<u8>>),
    AssertFn: FnOnce(&mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, ()>,
{
    rt.block_on(async {
        while logger.pop().is_some() {}

        let (dlq_tx, dlq_rx) = mpsc::channel(1);
        let mut dlq_rx = Some(dlq_rx);
        setup(&dlq_tx, &mut dlq_rx);
        let (mut queues, handle) =
            PushQueues::bounded_with_rate_dlq(1, 1, None, Some(dlq_tx)).unwrap();

        handle.push_high_priority(1u8).await.unwrap();
        handle.try_push(2u8, PushPriority::High, policy).unwrap();

        let (_, val) = queues.recv().await.unwrap();
        assert_eq!(val, 1);

        assertion(&mut dlq_rx).await;

        let mut found_error = false;
        while let Some(record) = logger.pop() {
            if record.level() == log::Level::Error {
                assert!(record.args().contains(expected));
                found_error = true;
                break;
            }
        }
        assert!(found_error, "error log not found");
    });
}
