//! Shared harness for the dead-letter queue drop-logging tests.
//!
//! Both the warning-content tests in `push_dlq_logging` and the cadence tests
//! in `push_dlq_cadence` need a queue whose dead-letter sink refuses a dropped
//! frame, and a dropped frame whose `Debug` output is recognizable. Keeping
//! that setup here stops the two suites from drifting apart while leaving each
//! test module within the repository's file-length limit.
//!
//! This file is not a test target: suites include it with `#[path]`.

use std::{io, time::Duration};

use tokio::sync::mpsc;
use wireframe::push::{PushHandle, PushPolicy, PushPriority, PushQueues};
use wireframe_testing::LoggerHandle;

/// Marker carried by the dropped frame; it must never reach a log record.
pub(crate) const DROPPED_SECRET: &str = "dlq-dropped-secret-7f3c1a";

/// Marker carried by the frame that preloads a full dead-letter queue.
pub(crate) const PRELOADED_FRAME: &str = "dlq-preloaded-frame-2b9e4d";

/// Frame whose derived `Debug` would expose its payload verbatim.
///
/// This is the hazard the regression guards against: any warning that formats
/// the frame would spell out [`DROPPED_SECRET`] in the log.
#[derive(Debug)]
pub(crate) struct SecretFrame {
    /// Payload marker that must stay out of logs.
    payload: &'static str,
}

/// How the dead-letter queue refuses the dropped frame.
#[derive(Clone, Copy)]
pub(crate) enum DlqFailure {
    /// The DLQ receiver is alive but the channel has no free capacity.
    Full,
    /// The DLQ receiver has been dropped.
    Closed,
}

/// Built queues whose DLQ is arranged to refuse the next dropped frame.
pub(crate) struct RefusingDlq {
    /// Receiver side of the push queues; dropping it would close them.
    _queues: PushQueues<SecretFrame>,
    /// Handle used to attempt the push.
    pub(crate) handle: PushHandle<SecretFrame>,
    /// Keeps a full DLQ open; `None` means its receiver was dropped.
    _dlq_rx: Option<mpsc::Receiver<SecretFrame>>,
}

/// Drain the logger into owned records for after-the-fact assertions.
pub(crate) fn drain_records(logger: &mut LoggerHandle) -> Vec<(log::Level, String)> {
    let mut records = Vec::new();
    while let Some(record) = logger.pop() {
        records.push((record.level(), record.args().to_owned()));
    }
    records
}

/// Build queues and arrange for the DLQ to refuse the next dropped frame.
///
/// The push queue selected by `priority` is filled first, so a dropping policy
/// reaches the dead-letter hand-off instead of succeeding. `every_n` and
/// `interval` are the drop-log cadence the caller wants observed.
pub(crate) async fn refusing_dlq(
    failure: DlqFailure,
    priority: PushPriority,
    every_n: usize,
    interval: Duration,
) -> io::Result<RefusingDlq> {
    let (dlq_tx, dlq_rx) = mpsc::channel(1);
    let dlq_receiver = match failure {
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
    let (queues, handle) = super::support::builder::<SecretFrame>()
        .unlimited()
        .dlq(Some(dlq_tx))
        .dlq_log_every_n(every_n)
        .dlq_log_interval(interval)
        .build()
        .map_err(|e| io::Error::other(format!("build queues failed: {e}")))?;
    fill_target_queue(&handle, priority).await?;
    Ok(RefusingDlq {
        _queues: queues,
        handle,
        _dlq_rx: dlq_receiver,
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
pub(crate) fn push_secret_frame(
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
