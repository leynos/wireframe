//! Regression tests for the content of dead-letter queue drop warnings.
//!
//! A warning must describe why a frame was lost without disclosing the frame
//! itself: these tests plant a secret marker in a frame whose derived `Debug`
//! would print it verbatim and assert that no captured log record ever
//! contains it. The cadence tests — the count threshold and the minimum
//! interval that decide when the aggregated loss is reported — live in
//! `push_dlq_cadence`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

mod support;

#[path = "common/push_dlq_support.rs"]
mod push_dlq_support;

use std::{io, time::Duration};

use push_dlq_support::{
    DROPPED_SECRET,
    DlqFailure,
    PRELOADED_FRAME,
    push_secret_frame,
    refusing_dlq,
};
use rstest::rstest;
use serial_test::serial;
use wireframe::push::{PushPolicy, PushPriority};
use wireframe_testing::{LoggerHandle, TestResult, logger};

/// The builder's default interval, kept so the content tests log every drop.
const DEFAULT_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// The `trigger_dlq_state` field expected in the loss warning.
fn expected_state(failure: DlqFailure) -> &'static str {
    match failure {
        DlqFailure::Full => "trigger_dlq_state=Full",
        DlqFailure::Closed => "trigger_dlq_state=Closed",
    }
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
    require_field(warning, expected_state(failure))?;
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
        let records = push_dlq_support::drain_records(&mut logger);
        assert_single_safe_warning(&records, failure, priority, policy)
    })?;
    Ok(())
}
