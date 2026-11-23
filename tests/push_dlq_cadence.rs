//! Cadence tests for dead-letter queue drop warnings.
//!
//! The count threshold and the minimum interval decide when an aggregated loss
//! is reported and how many losses each report stands for. The warning-content
//! tests, which prove that a warning never discloses the dropped frame, live in
//! `push_dlq_logging`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

mod support;

#[path = "common/push_dlq_support.rs"]
mod push_dlq_support;

use std::{
    io,
    time::{Duration, Instant},
};

use push_dlq_support::{DlqFailure, SecretFrame, drain_records, push_secret_frame, refusing_dlq};
use rstest::rstest;
use serial_test::serial;
use wireframe::push::{PushHandle, PushPolicy, PushPriority};
use wireframe_testing::{LoggerHandle, TestResult, logger};

/// Long enough that interval-based logging cannot fire during a test.
const LONG_INTERVAL: Duration = Duration::from_secs(3600);

/// Comfortably longer than the fifty-millisecond interval the cadence test
/// sets, so the check cannot depend on how quickly the test reaches it.
const PAST_INTERVAL: Duration = Duration::from_millis(100);

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

/// Whether the second loss's reports are consistent with the measured gap.
///
/// Inside the interval the only correct outcome is silence. A descheduled run
/// may instead have reported that loss on its own, which must then be its only
/// report.
fn is_plausible_second_loss(counts: &[usize], is_inside_interval: bool) -> bool {
    if is_inside_interval {
        return counts.is_empty();
    }
    counts.is_empty() || counts == [1]
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
/// Three properties share one run. The first loss reports; a loss arriving
/// while the interval has not expired is held back rather than logged; and the
/// next loss after the interval reports again — proving the interval is
/// measured from the last report, not from some earlier instant. The second
/// report aggregates the held-back loss with its own trigger, so a counter the
/// first report failed to clear would show up as three. Interval 50 ms against
/// 100 ms of sleep leaves a 2:1 margin on both sides of each boundary.
///
/// The suppression check is the one observation a descheduled run cannot make:
/// if the process pauses for most of an interval between the first report and
/// the second loss, that loss correctly reports and the check would fail a
/// correct implementation. The test times the window itself and only asserts
/// suppression when it is provably inside the interval; otherwise it records
/// what happened and derives the next expectation from it, so a busy host
/// cannot turn the test red.
///
/// The `>` versus `>=` boundary between `elapsed()` and the configured
/// interval has no test. Equality is possible in principle, but forcing an
/// elapsed time exactly equal to the interval to nanosecond precision needs a
/// controllable clock this test does not have, so this test cannot distinguish
/// the two operators. The mutant is an accepted limitation of this test, not an
/// equivalence.
#[rstest]
#[serial(push_policies)]
fn dlq_drop_logging_reports_each_interval(mut logger: LoggerHandle) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let interval = Duration::from_millis(50);
        let refusing = refusing_dlq(DlqFailure::Full, PushPriority::High, 0, interval).await?;
        logger.clear();

        // First loss: the timestamp starts at construction, so one interval of
        // sleep is well past it and the report fires. The clock is read before
        // the loss so the window measured below contains the production
        // observation rather than starting after it.
        tokio::time::sleep(PAST_INTERVAL).await;
        let reported_at = Instant::now();
        lose_frame(&refusing.handle, PushPriority::High)?;
        require_drop_counts(&mut logger, &[1])?;

        // Second loss with the interval still running. Half an interval of
        // slack covers the few microseconds before the loss itself, so a gap
        // below the threshold proves suppression.
        lose_frame(&refusing.handle, PushPriority::High)?;
        let second = drop_counts(&drain_records(&mut logger))?;
        let is_inside_interval = reported_at.elapsed() < interval / 2;
        let was_suppressed = second.is_empty();
        if !is_plausible_second_loss(&second, is_inside_interval) {
            return Err(io::Error::other(format!(
                "second loss reported {second:?} with the interval {}",
                if is_inside_interval {
                    "still running"
                } else {
                    "already expired"
                }
            ))
            .into());
        }

        // Third loss after another full interval: the report stands for every
        // loss the first report left in the counter plus this trigger, so a
        // counter the first report failed to clear would show up as three.
        tokio::time::sleep(PAST_INTERVAL).await;
        lose_frame(&refusing.handle, PushPriority::High)?;
        require_drop_counts(&mut logger, &[usize::from(was_suppressed) + 1])?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}
