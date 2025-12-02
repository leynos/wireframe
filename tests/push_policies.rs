#![cfg(not(loom))]
//! Tests for push queue policy behaviour.

mod support;

use futures::{FutureExt, future::BoxFuture};
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::{runtime::Runtime, sync::mpsc};
use wireframe::push::{PushPolicy, PushPriority, PushQueuesBuilder};
use wireframe_testing::{LoggerHandle, logger};

/// Builds a single-thread [`Runtime`] for async tests.
#[fixture]
fn rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build test runtime")
}

#[expect(
    unused_braces,
    reason = "rustc false positive for single-line rstest fixtures"
)]
// allow(unfulfilled_lint_expectations): rustc occasionally fails to emit the expected
// lint for single-line rstest fixtures on stable.
#[allow(unfulfilled_lint_expectations)]
#[fixture]
fn builder() -> PushQueuesBuilder<u8> { support::builder::<u8>() }

#[derive(Clone, Copy)]
struct PolicyCase {
    policy: PushPolicy,
    expect_warning: bool,
    expected_msg: &'static str,
}

type DlqSetup = fn(&mpsc::Sender<u8>, &mut Option<mpsc::Receiver<u8>>);
type DlqAssertion = for<'a> fn(&'a mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'a, ()>;

#[derive(Clone, Copy)]
struct DlqCase {
    setup: DlqSetup,
    policy: PushPolicy,
    assertion: DlqAssertion,
    expected: &'static str,
}

/// Verifies how queue policies log and drop when the queue is full.
#[rstest]
#[case::drop_if_full(PolicyCase { policy: PushPolicy::DropIfFull, expect_warning: false, expected_msg: "push queue full" })]
#[case::warn_and_drop(PolicyCase { policy: PushPolicy::WarnAndDropIfFull, expect_warning: true, expected_msg: "push queue full" })]
#[serial(push_policies)]
fn push_policy_behaviour(
    rt: Runtime,
    mut logger: LoggerHandle,
    builder: PushQueuesBuilder<u8>,
    #[case] case: PolicyCase,
) {
    let PolicyCase {
        policy,
        expect_warning,
        expected_msg,
    } = case;
    rt.block_on(async {
        while logger.pop().is_some() {}
        let (mut queues, handle) = builder.build().expect("failed to build PushQueues");

        handle
            .push_high_priority(1u8)
            .await
            .expect("push high priority failed");
        handle
            .try_push(2u8, PushPriority::High, policy)
            .expect("try_push failed");

        let (_, val) = queues.recv().await.expect("recv failed");
        assert_eq!(val, 1);
        assert!(
            queues.recv().now_or_never().is_none(),
            "queue should be empty"
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

/// Dropped frames are forwarded to the dead letter queue.
#[rstest]
fn dropped_frame_goes_to_dlq(rt: Runtime, builder: PushQueuesBuilder<u8>) {
    rt.block_on(async {
        let (dlq_tx, mut dlq_rx) = mpsc::channel(1);
        let (mut queues, handle) = builder
            .unlimited()
            .dlq(Some(dlq_tx))
            .build()
            .expect("failed to build PushQueues");

        handle
            .push_high_priority(1u8)
            .await
            .expect("push high priority failed");
        handle
            .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
            .expect("try_push failed");

        let (_, val) = queues.recv().await.expect("recv failed");
        assert_eq!(val, 1);
        assert_eq!(dlq_rx.recv().await.expect("dlq recv failed"), 2);
    });
}

/// Preloads the DLQ to simulate a full queue.
fn fill_dlq(tx: &mpsc::Sender<u8>, _rx: &mut Option<mpsc::Receiver<u8>>) {
    tx.try_send(99).expect("send failed");
}

/// Drops the receiver to simulate a closed DLQ channel.
fn close_dlq(_: &mpsc::Sender<u8>, rx: &mut Option<mpsc::Receiver<u8>>) { drop(rx.take()); }

/// Asserts that one message is queued and the DLQ then reports empty.
fn assert_dlq_full(rx: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, ()> {
    Box::pin(async move {
        let receiver = rx.as_mut().expect("receiver missing");
        assert_eq!(receiver.recv().await.expect("dlq recv failed"), 99);
        assert!(receiver.try_recv().is_err());
    })
}

/// Confirms no receiver is present when the DLQ is closed.
fn assert_dlq_closed(_: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, ()> { Box::pin(async {}) }

/// Parameterised checks for error logs when DLQ interactions fail.
#[rstest]
#[case::dlq_full(
    DlqCase {
        setup: fill_dlq,
        policy: PushPolicy::WarnAndDropIfFull,
        assertion: assert_dlq_full,
        expected: "DLQ dropped frames"
    }
)]
#[case::dlq_closed(
    DlqCase {
        setup: close_dlq,
        policy: PushPolicy::DropIfFull,
        assertion: assert_dlq_closed,
        expected: "DLQ dropped frames"
    }
)]
#[serial(push_policies)]
fn dlq_error_scenarios(
    rt: Runtime,
    mut logger: LoggerHandle,
    #[case] case: DlqCase,
    builder: PushQueuesBuilder<u8>,
) {
    rt.block_on(async {
        while logger.pop().is_some() {}

        let DlqCase {
            setup,
            policy,
            assertion,
            expected,
        } = case;
        let (dlq_tx, dlq_rx) = mpsc::channel(1);
        let mut dlq_rx = Some(dlq_rx);
        setup(&dlq_tx, &mut dlq_rx);
        let (mut queues, handle) = builder
            .unlimited()
            .dlq(Some(dlq_tx))
            .build()
            .expect("failed to build PushQueues");

        handle
            .push_high_priority(1u8)
            .await
            .expect("push high priority failed");
        handle
            .try_push(2u8, PushPriority::High, policy)
            .expect("try_push failed");

        let (_, val) = queues.recv().await.expect("recv failed");
        assert_eq!(val, 1);

        assertion(&mut dlq_rx).await;

        let mut found = false;
        while let Some(record) = logger.pop() {
            if record.level() == log::Level::Warn && record.args().contains(expected) {
                found = true;
            }
        }
        assert!(found, "expected DLQ warning log missing");
    });
}
