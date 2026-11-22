//! Property coverage for count-based dead-letter drop reporting.
//!
//! The example cases in `push_dlq_cadence` pin a handful of thresholds and
//! loss counts. This suite states the invariant they illustrate once and
//! checks it across the input space: after any number of losses, the counts
//! the warnings reported plus the losses still pending are exactly the losses
//! that occurred, and every report stands for exactly one cadence's worth.
//!
//! Timing is deliberately absent. Interval-triggered reporting needs a
//! controllable clock, which this crate does not have, so it stays covered by
//! the deterministic cases in `push_dlq_cadence`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

mod support;

#[path = "common/push_dlq_support.rs"]
mod push_dlq_support;

use std::{io, time::Duration};

use proptest::prelude::*;
use push_dlq_support::{DlqFailure, SecretFrame, drain_records, push_secret_frame, refusing_dlq};
use serial_test::serial;
use wireframe::push::{PushHandle, PushPolicy, PushPriority};
use wireframe_testing::{LoggerHandle, TestResult, logger};

/// Long enough that interval-based logging cannot fire during a case, so only
/// the count threshold decides when a report is emitted.
const LONG_INTERVAL: Duration = Duration::from_secs(3600);

/// The largest cadence sampled. Beyond this, a case spends most of its time
/// losing frames without exercising any new aggregation behaviour.
const MAX_CADENCE: usize = 8;

/// The largest loss count sampled, kept above `MAX_CADENCE` so several
/// reports can fall inside one case.
const MAX_LOSSES: usize = 24;

/// Push one frame the full queue cannot accept, losing it to the DLQ.
fn lose_frame(handle: &PushHandle<SecretFrame>) -> io::Result<()> {
    push_secret_frame(handle, PushPriority::High, PushPolicy::DropIfFull)
}

/// Extract the `dropped=` count from every DLQ loss warning, in capture order.
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

/// Check the conservation rule for one cadence and loss count.
///
/// With no interval in play, a report fires on every `cadence`-th loss, so the
/// reports must be `cadence` each and the unreported remainder is
/// `losses % cadence`. Summing the reports and that remainder has to recover
/// every lost frame: a counter that is incremented wrongly, cleared early, or
/// left uncleared breaks the total.
fn check_conservation(losses: usize, cadence: usize, counts: &[usize]) -> Result<(), String> {
    // The generated cadence is at least one, so the checked forms only make
    // the division explicit; a zero would already be a broken strategy.
    let Some(expected_reports) = losses.checked_div(cadence) else {
        return Err(format!("cadence {cadence} cannot divide {losses} losses"));
    };
    if counts.len() != expected_reports {
        return Err(format!(
            "{losses} losses at cadence {cadence} should report {expected_reports} times, not {}",
            counts.len()
        ));
    }
    if !counts.iter().all(|count| *count == cadence) {
        return Err(format!(
            "every report must stand for one cadence: {counts:?}"
        ));
    }
    let reported: usize = counts.iter().sum();
    let Some(pending) = losses.checked_rem(cadence) else {
        return Err(format!("cadence {cadence} cannot divide {losses} losses"));
    };
    if reported + pending != losses {
        return Err(format!(
            "reported ({reported}) plus pending ({pending}) must equal the {losses} losses"
        ));
    }
    Ok(())
}

/// Run one case: build a refusing DLQ at `cadence`, lose `losses` frames, and
/// check the reports conserve them.
fn run_case(
    mut logger: LoggerHandle,
    failure: DlqFailure,
    cadence: usize,
    losses: usize,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let refusing = refusing_dlq(failure, PushPriority::High, cadence, LONG_INTERVAL).await?;
        logger.clear();
        for _ in 0..losses {
            lose_frame(&refusing.handle)?;
        }
        let counts = drop_counts(&drain_records(&mut logger))?;
        check_conservation(losses, cadence, &counts).map_err(io::Error::other)
    })?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    /// Reports conserve every lost frame, whether the DLQ is full or closed.
    #[test]
    #[serial(push_policies)]
    fn reported_counts_conserve_the_losses(
        cadence in 1usize..=MAX_CADENCE,
        losses in 0usize..=MAX_LOSSES,
        is_closed in any::<bool>(),
    ) {
        let failure = if is_closed { DlqFailure::Closed } else { DlqFailure::Full };
        let logger = logger();
        run_case(logger, failure, cadence, losses)
            .map_err(|e| TestCaseError::fail(format!("cadence {cadence}, losses {losses}: {e}")))?;
    }
}
