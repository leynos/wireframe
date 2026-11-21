//! Regression tests for dead-letter queue drop warnings.
//!
//! Two properties are covered. The *content* of a warning must describe why a
//! frame was lost without disclosing the frame itself: these tests plant a
//! secret marker in a frame whose derived `Debug` would print it verbatim and
//! assert that no captured log record ever contains it. The *cadence* — the
//! count threshold and the minimum interval — decides when the aggregated loss
//! is reported and how many losses each report stands for.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

mod support;

use std::{io, time::Duration};

use rstest::rstest;
use serial_test::serial;
use tokio::sync::mpsc;
use wireframe::push::{PushHandle, PushPolicy, PushPriority, PushQueues};
use wireframe_testing::{LoggerHandle, TestResult, logger};

/// The builder's default interval, kept so the content tests log every drop.
const DEFAULT_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// Long enough that interval-based logging cannot fire during a test.
const LONG_INTERVAL: Duration = Duration::from_secs(3600);

/// Comfortably longer than the one-millisecond interval the reset test sets.
const PAST_SHORT_INTERVAL: Duration = Duration::from_millis(20);

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
/// reaches the dead-letter hand-off instead of succeeding. `every_n` and
/// `interval` are the drop-log cadence the caller wants observed.
async fn refusing_dlq(
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
    let (queues, handle) = support::builder::<SecretFrame>()
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

/// Return the single DLQ loss warning, or explain why there is not exactly one.
fn single_dlq_warning(records: &[(log::Level, String)]) -> io::Result<&str> {
    let mut warnings = records
        .iter()
        .filter(|record| record.0 == log::Level::Warn && record.1.contains("DLQ dropped frames"))
        .map(|record| record.1.as_str());
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
    Ok(warning)
}

/// Require `warning` to contain `expected`, naming both on failure.
fn require_field(warning: &str, expected: &str) -> io::Result<()> {
    if warning.contains(expected) {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "DLQ warning missing {expected}: {warning}"
    )))
}

/// Assert that no captured record contains either frame marker.
fn assert_no_marker_leaked(records: &[(log::Level, String)]) -> io::Result<()> {
    let Some((_, text)) = records
        .iter()
        .find(|(_, text)| text.contains(DROPPED_SECRET) || text.contains(PRELOADED_FRAME))
    else {
        return Ok(());
    };
    Err(io::Error::other(format!(
        "DLQ warning disclosed frame contents: {text}"
    )))
}

/// Assert there is exactly one DLQ warning, naming the expected state,
/// priority and policy, and that no record leaks either marker.
fn assert_single_safe_warning(
    records: &[(log::Level, String)],
    failure: DlqFailure,
    priority: PushPriority,
    policy: PushPolicy,
) -> io::Result<()> {
    let warning = single_dlq_warning(records)?;
    // The trigger metadata identifies the frame that fired this report, so a
    // warning attributed to the wrong priority or policy would mislead an
    // operator even though it leaked no payload.
    require_field(warning, failure.expected_state())?;
    require_field(warning, &format!("trigger_priority={priority:?}"))?;
    require_field(warning, &format!("trigger_policy={policy:?}"))?;
    assert_no_marker_leaked(records)
}

/// A dropped frame's `Debug` output must never appear in the DLQ warning.
///
/// Covers both ways a configured DLQ can refuse a frame, both priorities and
/// both dropping policies. In every combination the warning must name the
/// dead-letter state, priority and policy that fired it, and stay silent about
/// the payload.
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
        logger.clear();
        let refusing = refusing_dlq(failure, priority, 1, DEFAULT_LOG_INTERVAL).await?;
        push_secret_frame(&refusing.handle, priority, policy)?;
        let records = drain_records(&mut logger);
        assert_single_safe_warning(&records, failure, priority, policy)
    })?;
    Ok(())
}

/// Push one frame the full queue cannot accept, losing it to the full DLQ.
fn lose_frame(handle: &PushHandle<SecretFrame>, priority: PushPriority) -> io::Result<()> {
    push_secret_frame(handle, priority, PushPolicy::DropIfFull)
}

/// Extract the `dropped=` count from every DLQ loss warning, in capture order.
///
/// Parsing rather than substring matching proves the diagnostic reports how
/// many losses its trigger stood for. A warning missing the field is an error
/// to surface, not a record to skip.
fn drop_counts(records: &[(log::Level, String)]) -> io::Result<Vec<usize>> {
    records
        .iter()
        .filter(|(level, text)| *level == log::Level::Warn && text.contains("DLQ dropped frames"))
        .map(|(_, text)| {
            text.split("dropped=")
                .nth(1)
                .and_then(|rest| rest.split(',').next())
                .ok_or_else(|| io::Error::other(format!("DLQ warning lacks dropped: {text}")))?
                .trim()
                .parse::<usize>()
                .map_err(|e| io::Error::other(format!("unparsable dropped count: {e}")))
        })
        .collect()
}

/// Require the logger to hold exactly the expected sequence of drop counts.
fn require_drop_counts(logger: &mut LoggerHandle, expected: &[usize]) -> io::Result<()> {
    let actual = drop_counts(&drain_records(logger))?;
    if actual == expected {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "expected DLQ drop counts {expected:?}, found {actual:?}"
    )))
}

/// Count-based logging waits for the cadence and reports every loss it
/// aggregated, whether the dead-letter queue is full or closed.
#[rstest]
#[case::full(DlqFailure::Full)]
#[case::closed(DlqFailure::Closed)]
#[serial(push_policies)]
fn dlq_drop_logging_waits_for_the_count_threshold(
    mut logger: LoggerHandle,
    #[case] failure: DlqFailure,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let refusing = refusing_dlq(failure, PushPriority::High, 3, LONG_INTERVAL).await?;
        logger.clear();

        lose_frame(&refusing.handle, PushPriority::High)?;
        lose_frame(&refusing.handle, PushPriority::High)?;
        require_drop_counts(&mut logger, &[])?;

        lose_frame(&refusing.handle, PushPriority::High)?;
        require_drop_counts(&mut logger, &[3])?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}

/// A zero cadence disables count-based logging, and a long interval leaves the
/// diagnostic silent however many frames are lost.
#[rstest]
#[serial(push_policies)]
fn dlq_drop_logging_can_be_silenced(mut logger: LoggerHandle) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let refusing = refusing_dlq(DlqFailure::Full, PushPriority::High, 0, LONG_INTERVAL).await?;
        logger.clear();
        for _ in 0..3 {
            lose_frame(&refusing.handle, PushPriority::High)?;
        }
        require_drop_counts(&mut logger, &[])?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}

/// Interval-based logging reports each interval's losses and then resets, so
/// the next interval reports only its own.
///
/// The `>` versus `>=` boundary between `elapsed()` and the configured
/// interval has no test. Separating them needs an elapsed time equal to the
/// interval to nanosecond precision, which no real clock yields on demand, so
/// every observable duration sits strictly on one side. The mutant is an
/// accepted survivor, not an equivalence.
#[rstest]
#[serial(push_policies)]
fn dlq_drop_logging_reports_each_interval(mut logger: LoggerHandle) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let interval = Duration::from_millis(1);
        let refusing = refusing_dlq(DlqFailure::Full, PushPriority::High, 0, interval).await?;
        logger.clear();

        tokio::time::sleep(PAST_SHORT_INTERVAL).await;
        lose_frame(&refusing.handle, PushPriority::High)?;
        require_drop_counts(&mut logger, &[1])?;

        tokio::time::sleep(PAST_SHORT_INTERVAL).await;
        lose_frame(&refusing.handle, PushPriority::High)?;
        require_drop_counts(&mut logger, &[1])?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}
