#![cfg_attr(loom, allow(missing_docs))]
#![cfg(not(loom))]
//! Tests for push queue policy behaviour.

mod support;

use std::io;

use futures::{FutureExt, future::BoxFuture};
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::sync::mpsc;
use wireframe::push::{PushPolicy, PushPriority, PushQueuesBuilder};
use wireframe_testing::{LoggerHandle, logger};

mod common;
use common::TestResult;

#[fixture]
fn builder() -> PushQueuesBuilder<u8> {
    // PushQueuesBuilder for push queue policy tests
    support::builder::<u8>()
}

#[derive(Clone, Copy)]
struct PolicyCase {
    policy: PushPolicy,
    expect_warning: bool,
    expected_msg: &'static str,
}

type DlqSetup = fn(&mpsc::Sender<u8>, &mut Option<mpsc::Receiver<u8>>) -> TestResult<()>;
type DlqAssertion = for<'a> fn(&'a mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'a, TestResult<()>>;

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
    mut logger: LoggerHandle,
    builder: PushQueuesBuilder<u8>,
    #[case] case: PolicyCase,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let PolicyCase {
        policy,
        expect_warning,
        expected_msg,
    } = case;
    rt.block_on(async move {
        while logger.pop().is_some() {}
        let (mut queues, handle) = builder
            .build()
            .map_err(|e| io::Error::other(format!("build queues failed: {e}")))?;

        handle
            .push_high_priority(1u8)
            .await
            .map_err(|e| io::Error::other(format!("push high priority failed: {e}")))?;
        handle
            .try_push(2u8, PushPriority::High, policy)
            .map_err(|e| io::Error::other(format!("try_push failed: {e}")))?;

        let (_, val) = queues
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "recv failed"))?;
        if val != 1 {
            return Err(io::Error::other("unexpected value dequeued").into());
        }
        if queues.recv().now_or_never().is_some() {
            return Err(io::Error::other("queue should be empty").into());
        }

        let mut found_warning = false;
        while let Some(record) = logger.pop() {
            if record.level() == log::Level::Warn && record.args().contains(expected_msg) {
                found_warning = true;
            }
        }

        if expect_warning {
            if !found_warning {
                return Err(io::Error::other("warning log not found").into());
            }
        } else if found_warning {
            return Err(io::Error::other("unexpected warning log found").into());
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}

/// Dropped frames are forwarded to the dead letter queue.
#[rstest]
fn dropped_frame_goes_to_dlq(builder: PushQueuesBuilder<u8>) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let (dlq_tx, mut dlq_rx) = mpsc::channel(1);
        let (mut queues, handle) = builder
            .unlimited()
            .dlq(Some(dlq_tx))
            .build()
            .map_err(|e| io::Error::other(format!("build queues failed: {e}")))?;

        handle
            .push_high_priority(1u8)
            .await
            .map_err(|e| io::Error::other(format!("push high priority failed: {e}")))?;
        handle
            .try_push(2u8, PushPriority::High, PushPolicy::DropIfFull)
            .map_err(|e| io::Error::other(format!("try_push failed: {e}")))?;

        let (_, val) = queues
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "recv failed"))?;
        if val != 1 {
            return Err(io::Error::other("unexpected dequeued value").into());
        }
        let dlq_val = dlq_rx
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "dlq recv failed"))?;
        if dlq_val != 2 {
            return Err(io::Error::other("unexpected DLQ value").into());
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}

/// Preloads the DLQ to simulate a full queue.
fn fill_dlq(tx: &mpsc::Sender<u8>, _rx: &mut Option<mpsc::Receiver<u8>>) -> TestResult<()> {
    tx.try_send(99)
        .map_err(|e| io::Error::other(format!("send failed: {e}")))?;
    Ok(())
}

/// Drops the receiver to simulate a closed DLQ channel.
fn close_dlq(_: &mpsc::Sender<u8>, rx: &mut Option<mpsc::Receiver<u8>>) -> TestResult<()> {
    if rx.is_none() {
        return Err("DLQ receiver missing".into());
    }
    drop(rx.take());
    Ok(())
}

/// Asserts that one message is queued and the DLQ then reports empty.
fn assert_dlq_full(rx: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, TestResult<()>> {
    Box::pin(async move {
        let receiver = rx
            .as_mut()
            .ok_or_else(|| io::Error::other("receiver missing"))?;
        let value = receiver
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "dlq recv failed"))?;
        if value != 99 {
            return Err(io::Error::other("unexpected DLQ value").into());
        }
        if receiver.try_recv().is_ok() {
            return Err(io::Error::other("expected DLQ to be empty").into());
        }
        Ok(())
    })
}

/// Confirms no receiver is present when the DLQ is closed.
fn assert_dlq_closed(_: &mut Option<mpsc::Receiver<u8>>) -> BoxFuture<'_, TestResult<()>> {
    Box::pin(async { Ok(()) })
}

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
    mut logger: LoggerHandle,
    #[case] case: DlqCase,
    builder: PushQueuesBuilder<u8>,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        while logger.pop().is_some() {}

        let DlqCase {
            setup,
            policy,
            assertion,
            expected,
        } = case;
        let (dlq_tx, dlq_rx) = mpsc::channel(1);
        let mut dlq_rx = Some(dlq_rx);
        setup(&dlq_tx, &mut dlq_rx)
            .map_err(|e| io::Error::other(format!("DLQ setup failed: {e}")))?;
        let (mut queues, handle) = builder
            .unlimited()
            .dlq(Some(dlq_tx))
            .build()
            .map_err(|e| io::Error::other(format!("build queues failed: {e}")))?;

        handle
            .push_high_priority(1u8)
            .await
            .map_err(|e| io::Error::other(format!("push high priority failed: {e}")))?;
        handle
            .try_push(2u8, PushPriority::High, policy)
            .map_err(|e| io::Error::other(format!("try_push failed: {e}")))?;

        let (_, val) = queues
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "recv failed"))?;
        if val != 1 {
            return Err(io::Error::other("unexpected dequeued value").into());
        }

        assertion(&mut dlq_rx)
            .await
            .map_err(|e| io::Error::other(format!("DLQ assertion failed: {e}")))?;

        let mut found = false;
        while let Some(record) = logger.pop() {
            if record.level() == log::Level::Warn && record.args().contains(expected) {
                found = true;
            }
        }
        if !found {
            return Err(io::Error::other("expected DLQ warning log missing").into());
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}
