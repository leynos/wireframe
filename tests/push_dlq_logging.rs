//! Regression tests for the content of dead-letter queue drop warnings.
//!
//! The DLQ loss warning must describe *why* a frame was lost without
//! disclosing the frame itself. These tests plant a secret marker in a frame
//! whose derived `Debug` would print it verbatim and assert that no captured
//! log record ever contains that marker.
#![cfg(not(loom))]

mod support;

use std::io;

use rstest::rstest;
use serial_test::serial;
use tokio::sync::mpsc;
use wireframe::push::{PushHandle, PushPolicy, PushPriority, PushQueues};
use wireframe_testing::{LoggerHandle, TestResult, logger};

/// Marker carried by the dropped frame; it must never reach a log record.
const DROPPED_SECRET: &str = "dlq-dropped-secret-7f3c1a";

/// Marker carried by the frame that preloads a full dead-letter queue.
const PRELOADED_FRAME: &str = "dlq-preloaded-frame-2b9e4d";

/// Frame whose derived `Debug` would expose its payload verbatim.
///
/// This is the hazard the regression guards against: any warning that formats
/// the frame would spell out [`DROPPED_SECRET`] in the log.
#[derive(Debug)]
struct SecretFrame {
    /// Payload marker that must stay out of logs.
    payload: &'static str,
}

/// How the dead-letter queue refuses the dropped frame.
#[derive(Clone, Copy)]
enum DlqFailure {
    /// The DLQ receiver is alive but the channel has no free capacity.
    Full,
    /// The DLQ receiver has been dropped.
    Closed,
}

impl DlqFailure {
    /// The `trigger_dlq_state` field expected in the loss warning.
    fn expected_state(self) -> &'static str {
        match self {
            Self::Full => "trigger_dlq_state=Full",
            Self::Closed => "trigger_dlq_state=Closed",
        }
    }
}

/// Built queues whose DLQ is arranged to refuse the next dropped frame.
struct RefusingDlq {
    /// Receiver side of the push queues; dropping it would close them.
    _queues: PushQueues<SecretFrame>,
    /// Handle used to attempt the push.
    handle: PushHandle<SecretFrame>,
    /// Keeps a full DLQ open; `None` means its receiver was dropped.
    _dlq_rx: Option<mpsc::Receiver<SecretFrame>>,
}

/// Drain the logger into owned records for after-the-fact assertions.
fn drain_records(logger: &mut LoggerHandle) -> Vec<(log::Level, String)> {
    let mut records = Vec::new();
    while let Some(record) = logger.pop() {
        records.push((record.level(), record.args().to_owned()));
    }
    records
}

/// Build queues and arrange for the DLQ to refuse the next dropped frame.
///
/// The push queue selected by `priority` is filled first, so a dropping policy
/// reaches the dead-letter hand-off instead of succeeding.
async fn refusing_dlq(failure: DlqFailure, priority: PushPriority) -> io::Result<RefusingDlq> {
    let (dlq_tx, dlq_rx) = mpsc::channel(1);
    let dlq_rx = match failure {
        DlqFailure::Full => {
            dlq_tx
                .try_send(SecretFrame {
                    payload: PRELOADED_FRAME,
                })
                .map_err(|e| io::Error::other(format!("DLQ preload failed: {e}")))?;
            Some(dlq_rx)
        }
        DlqFailure::Closed => {
            drop(dlq_rx);
            None
        }
    };
    let (queues, handle) = support::builder::<SecretFrame>()
        .unlimited()
        .dlq(Some(dlq_tx))
        .build()
        .map_err(|e| io::Error::other(format!("build queues failed: {e}")))?;
    fill_target_queue(&handle, priority).await?;
    Ok(RefusingDlq {
        _queues: queues,
        handle,
        _dlq_rx: dlq_rx,
    })
}

/// Fill the push queue that the drop is targeted at.
async fn fill_target_queue(
    handle: &PushHandle<SecretFrame>,
    priority: PushPriority,
) -> io::Result<()> {
    let fill = SecretFrame {
        payload: "queued-frame",
    };
    let result = match priority {
        PushPriority::High => handle.push_high_priority(fill).await,
        PushPriority::Low => handle.push_low_priority(fill).await,
    };
    result.map_err(|e| io::Error::other(format!("push failed: {e}")))
}

/// Push the frame carrying the secret marker under the given policy.
fn push_secret_frame(
    handle: &PushHandle<SecretFrame>,
    priority: PushPriority,
    policy: PushPolicy,
) -> io::Result<()> {
    let dropped = SecretFrame {
        payload: DROPPED_SECRET,
    };
    // The regression is only meaningful while `Debug` would disclose the
    // marker; reading the field also keeps the payload genuinely used.
    if !format!("{dropped:?}").contains(dropped.payload) {
        return Err(io::Error::other("secret marker missing from Debug output"));
    }
    handle
        .try_push(dropped, priority, policy)
        .map_err(|e| io::Error::other(format!("try_push failed: {e}")))
}

/// Assert there is exactly one DLQ warning, naming the expected state, and
/// that no record leaks either marker.
fn assert_single_safe_warning(
    records: &[(log::Level, String)],
    failure: DlqFailure,
) -> io::Result<()> {
    let mut warnings = records
        .iter()
        .filter(|record| record.0 == log::Level::Warn && record.1.contains("DLQ dropped frames"));
    let Some(warning) = warnings.next() else {
        return Err(io::Error::other(format!(
            "expected one DLQ warning: {records:?}"
        )));
    };
    if warnings.next().is_some() {
        return Err(io::Error::other(format!(
            "expected exactly one DLQ warning: {records:?}"
        )));
    }
    if !warning.1.contains(failure.expected_state()) {
        return Err(io::Error::other(format!(
            "DLQ warning missing {}: {}",
            failure.expected_state(),
            warning.1
        )));
    }
    for (_, text) in records {
        if text.contains(DROPPED_SECRET) || text.contains(PRELOADED_FRAME) {
            return Err(io::Error::other(format!(
                "DLQ warning disclosed frame contents: {text}"
            )));
        }
    }
    Ok(())
}

/// A dropped frame's `Debug` output must never appear in the DLQ warning.
///
/// Covers both ways a configured DLQ can refuse a frame, both priorities and
/// both dropping policies. In every combination the warning must name the
/// dead-letter state and stay silent about the payload.
#[rstest]
#[case::full_high_drop(DlqFailure::Full, PushPriority::High, PushPolicy::DropIfFull)]
#[case::full_high_warn(DlqFailure::Full, PushPriority::High, PushPolicy::WarnAndDropIfFull)]
#[case::full_low_drop(DlqFailure::Full, PushPriority::Low, PushPolicy::DropIfFull)]
#[case::full_low_warn(DlqFailure::Full, PushPriority::Low, PushPolicy::WarnAndDropIfFull)]
#[case::closed_high_drop(DlqFailure::Closed, PushPriority::High, PushPolicy::DropIfFull)]
#[case::closed_high_warn(DlqFailure::Closed, PushPriority::High, PushPolicy::WarnAndDropIfFull)]
#[case::closed_low_drop(DlqFailure::Closed, PushPriority::Low, PushPolicy::DropIfFull)]
#[case::closed_low_warn(DlqFailure::Closed, PushPriority::Low, PushPolicy::WarnAndDropIfFull)]
#[serial(push_policies)]
fn dlq_loss_warning_omits_frame_contents(
    mut logger: LoggerHandle,
    #[case] failure: DlqFailure,
    #[case] priority: PushPriority,
    #[case] policy: PushPolicy,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        while logger.pop().is_some() {}
        let refusing = refusing_dlq(failure, priority).await?;
        push_secret_frame(&refusing.handle, priority, policy)?;
        let records = drain_records(&mut logger);
        assert_single_safe_warning(&records, failure)
    })?;
    Ok(())
}
