//! Tests for push queue policy behaviour.

use futures::future::BoxFuture;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::{
    runtime::Runtime,
    sync::mpsc,
    time::{Duration, timeout},
};
use wireframe::push::{PushPolicy, PushPriority, PushQueues};
use wireframe_testing::{LoggerHandle, logger};

/// Creates a single-threaded Tokio runtime with all features enabled for use in tests.
///
/// # Examples
///
/// ```no_run
/// let runtime = rt();
/// runtime.block_on(async {
///     // Run async test code here
/// });
/// ```
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

/// Tests the behaviour of push queue policies when attempting to push to a full queue.
///
/// Verifies that, depending on the specified `PushPolicy`, a warning log is emitted or not
/// when a push is dropped due to a full queue. Ensures only the first item is received from
/// the queue and that dropped items do not appear. Checks logger output for expected warning
/// messages based on the policy.
///
/// # Parameters
///
/// - `policy`: The push policy to apply when the queue is full.
/// - `expect_warning`: Whether a warning log is expected for the given policy.
/// - `expected_msg`: The message expected to appear in the warning log, if any.
///
/// # Examples
///
/// ```no_run
/// // Runs as part of the test suite; not intended for direct invocation.
/// ```
#[rstest]
#[case::drop_if_full(PushPolicy::DropIfFull, false, "push queue full")]
#[case::warn_and_drop(PushPolicy::WarnAndDropIfFull, true, "push queue full")]
#[serial(push_policies)]
fn push_policy_behaviour(
    rt: Runtime,
    mut logger: LoggerHandle,
    #[case] policy: PushPolicy,
    #[case] expect_warning: bool,
    #[case] expected_msg: &str,
) {
    rt.block_on(async {
        while logger.pop().is_some() {}
        let (mut queues, handle) = PushQueues::bounded(1, 1);

        handle.push_high_priority(1u8).await.unwrap();
        handle.try_push(2u8, PushPriority::High, policy).unwrap();

        let (_, val) = queues.recv().await.unwrap();
        assert_eq!(val, 1);
        assert!(
            timeout(Duration::from_millis(20), queues.recv())
                .await
                .is_err()
        );

        let mut found_warning = false;
        while let Some(record) = logger.pop() {
            if record.level() == log::Level::Warn && record.args().contains(expected_msg) {
                found_warning = true;
            }
        }

        if expect_warning {
            assert!(found_warning, "warning log not found");
        } else {
            assert!(!found_warning, "unexpected warning log found");
        }
    });
}

/// Tests that a dropped frame due to a full push queue is sent to the dead-letter queue (DLQ).
///
/// This test pushes two items into a bounded queue with DLQ enabled. The first item is accepted,
/// and the second, which overflows the queue, is routed to the DLQ. The test asserts that the
/// first item is received from the main queue and the dropped item is received from the DLQ.
///
/// # Examples
///
/// ```no_run
/// dropped_frame_goes_to_dlq(rt());
/// ```
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

/// Fills the dead-letter queue (DLQ) channel to simulate a full DLQ state by sending a dummy value.
///
/// This function is used in tests to ensure that subsequent attempts to send to the DLQ will fail due to capacity.
///
/// # Examples
///
/// ```no_run
/// let (tx, mut rx) = tokio::sync::mpsc::channel(1);
/// setup_dlq_full(&tx, &mut Some(rx));
/// // The DLQ channel is now full.
/// ```
fn setup_dlq_full(tx: &mpsc::Sender<u8>, _rx: &mut Option<mpsc::Receiver<u8>>) {
    tx.try_send(99).unwrap();
}

/// Simulates a closed dead-letter queue (DLQ) by dropping its receiver.
///
/// This function is used in tests to mimic the scenario where the DLQ channel
/// is closed and cannot receive any more messages.
///
/// # Examples
///
/// ```no_run
/// let (tx, mut rx) = tokio::sync::mpsc::channel(1);
/// setup_dlq_closed(&tx, &mut Some(rx));
/// // The DLQ receiver is now dropped (closed).
/// ```
fn setup_dlq_closed(_: &mpsc::Sender<u8>, rx: &mut Option<mpsc::Receiver<u8>>) { drop(rx.take()); }

/// Asserts that the DLQ receiver contains the expected pre-filled value and is then empty.
///
/// This function checks that the next message received from the provided DLQ receiver is
/// the value `99`, and that no further messages are available.
///
/// # Examples
///
/// ```no_run
/// use tokio::sync::mpsc;
/// # use your_crate::assert_dlq_full;
/// let (tx, mut rx) = mpsc::channel(1);
/// tx.blocking_send(99).unwrap();
/// let mut rx_opt = Some(rx);
/// tokio_test::block_on(assert_dlq_full(&mut rx_opt));
/// ```
fn assert_dlq_full(rx: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, ()> {
    Box::pin(async move {
        let receiver = rx.as_mut().expect("receiver missing");
        assert_eq!(receiver.recv().await.unwrap(), 99);
        assert!(receiver.try_recv().is_err());
    })
}

/// Returns a future that completes immediately, performing no assertions on a closed DLQ receiver.
///
/// This function is used in tests to represent scenarios where the dead-letter queue (DLQ)
/// receiver has been closed and no further validation is required.
///
/// # Examples
///
/// ```no_run
/// use tokio::sync::mpsc;
/// use futures::executor::block_on;
///
/// let mut rx: Option<mpsc::Receiver<u8>> = None;
/// block_on(assert_dlq_closed(&mut rx));
/// ```
fn assert_dlq_closed(_: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, ()> { Box::pin(async {}) }

/// Tests error handling when dropped items cannot be delivered to the dead-letter queue (DLQ).
///
/// This parameterised test covers scenarios where the DLQ is either full or closed. It verifies
/// that dropped items are handled according to the specified push policy, and that appropriate
/// error logs are emitted when the DLQ cannot accept new items.
///
/// # Parameters
/// - `setup`: Function to configure the DLQ state (full or closed) before the test.
/// - `policy`: The push policy to use when the main queue is full.
/// - `expected`: Substring expected to appear in the error log message.
/// - `assertion`: Function to assert the final state of the DLQ receiver.
///
/// # Examples
///
/// ```no_run
/// // Runs automatically as part of the test suite; not intended for direct invocation.
/// ```
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
