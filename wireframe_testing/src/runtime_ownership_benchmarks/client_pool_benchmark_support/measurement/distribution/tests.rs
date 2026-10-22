//! Coverage for the percentile arithmetic and the failure accounting.
//!
//! These are pure-value tests: they exercise the accumulation and the rank
//! calculation directly, so a defect in either is reported as a wrong
//! statistic rather than as an intermittent burst failure. What the sampling
//! loop does with a real pool — that a failed burst is counted and not retried
//! into a sample — is covered by the client-pool benchmark suite's own tests.

use std::time::Duration;

use super::{ContendedLatency, RETAINED_FAILURES};

/// A distribution holding one sample per value, in the given order.
fn latency_of_millis(values: &[u64]) -> ContendedLatency {
    let mut latency = ContendedLatency::default();
    for value in values {
        latency.push_sample(Duration::from_millis(*value));
    }
    latency
}

/// A distribution holding `count` failures and no samples.
fn latency_of_failures(count: usize) -> ContendedLatency {
    let mut latency = ContendedLatency::default();
    for index in 0..count {
        latency.push_failure(format!("burst {index} failed"));
    }
    latency
}

#[test]
fn an_empty_distribution_reports_no_statistics() {
    let latency = ContendedLatency::default();
    assert_eq!(latency.successes(), 0);
    assert_eq!(latency.failure_count(), 0);
    assert_eq!(latency.samples(), []);
    assert_eq!(latency.min(), None);
    assert_eq!(latency.max(), None);
    assert_eq!(latency.median(), None);
    assert_eq!(latency.p90(), None);
    assert_eq!(latency.mean(), None);
}

/// Insertion order must not survive into the stored samples.
///
/// The statistics are index lookups on an ascending vector, so a sample
/// appended out of order would silently shift every percentile rather than
/// produce an obviously wrong answer.
#[test]
fn samples_are_stored_ascending_however_they_arrive() {
    let latency = latency_of_millis(&[30, 10, 20]);
    let ascending = [
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(30),
    ];
    assert_eq!(latency.samples(), ascending);
}

#[test]
fn statistics_describe_the_ascending_sample_set() {
    let latency = latency_of_millis(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    assert_eq!(latency.successes(), 10);
    assert_eq!(latency.min(), Some(Duration::from_millis(10)));
    assert_eq!(latency.max(), Some(Duration::from_millis(100)));
    // Nearest rank on ten samples: the 5th value for the median, the 9th
    // for p90. An interpolating definition would give the same median here
    // and a different p90, which is why the boundary case below matters.
    assert_eq!(latency.median(), Some(Duration::from_millis(50)));
    assert_eq!(latency.p90(), Some(Duration::from_millis(90)));
    assert_eq!(latency.mean(), Some(Duration::from_millis(55)));
}

/// Rank is one-based and counted from the fastest sample.
///
/// The median of two samples is the lower of them and the median of three
/// is the middle one: this is what pins the rank-to-index shift, which is
/// the one place an off-by-one would produce a plausible-looking figure.
#[rstest::rstest]
#[case(&[10, 20], 10)]
#[case(&[10, 20, 30], 20)]
#[case(&[10], 10)]
fn median_is_the_nearest_rank_sample(#[case] values: &[u64], #[case] expected: u64) {
    assert_eq!(
        latency_of_millis(values).median(),
        Some(Duration::from_millis(expected))
    );
}

/// A percentile is an observed burst, never a value between two of them.
///
/// This is the negative control for the interpolation decision. On an
/// even-sized set an interpolating median is the midpoint, which here is
/// the arithmetic mean — a figure no burst took. The mean is asserted beside
/// it so the two are shown to be computed differently rather than to happen
/// to agree.
#[test]
fn percentiles_are_observed_samples_not_interpolations() {
    let latency = latency_of_millis(&[10, 20]);

    assert_eq!(latency.mean(), Some(Duration::from_millis(15)));
    assert_eq!(
        latency.median(),
        Some(Duration::from_millis(10)),
        "the lower sample is the nearest-rank median, not the 15 ms midpoint"
    );
}

/// Failures are counted in full but retained in bounded memory.
///
/// The count is the figure the baseline reports, so it must not saturate at
/// the retention limit; the messages exist to identify the defect, and a run
/// that fails thousands of bursts must not build a message per burst.
#[test]
fn failures_are_counted_in_full_and_retained_in_bounded_memory() {
    let over_limit = RETAINED_FAILURES + 3;
    let latency = latency_of_failures(over_limit);
    assert_eq!(latency.failure_count(), over_limit);
    assert_eq!(latency.failures().len(), RETAINED_FAILURES);
    assert_eq!(
        latency.failures().first(),
        Some(&"burst 0 failed".to_string()),
        "retention keeps the earliest messages, which name the first occurrence"
    );
}

/// Churn folds across the run rather than being replaced by the last burst.
///
/// `PoolTaskChurn::since` returns a delta, so a fold that assigned instead
/// of adding would report whichever burst happened to run last. Summing is
/// what makes the figure the run's cost rather than a sample of it.
#[test]
fn churn_is_the_sum_of_the_sampled_bursts() {
    use wireframe::client::PoolTaskChurn;

    let mut latency = ContendedLatency::default();
    latency.push_churn(PoolTaskChurn {
        scheduler_spawns: 2,
        capacity_waits: 16,
    });
    latency.push_churn(PoolTaskChurn {
        scheduler_spawns: 3,
        capacity_waits: 7,
    });

    assert_eq!(
        latency.churn(),
        PoolTaskChurn {
            scheduler_spawns: 5,
            capacity_waits: 23,
        }
    );
}

/// A failure contributes no sample and does not dilute the statistics.
///
/// The statistics describe the successful bursts alone; the failure count is
/// what says how much of the run they do not describe. Summing the two is
/// the reader's job, and this pins that they stay separate.
#[test]
fn failures_leave_the_statistics_describing_the_successes_alone() {
    let mut latency = latency_of_millis(&[10, 20]);
    let median_before = latency.median();
    for index in 0..5 {
        latency.push_failure(format!("burst {index} failed"));
    }

    assert_eq!(latency.successes(), 2);
    assert_eq!(latency.failure_count(), 5);
    assert_eq!(latency.median(), median_before);
    assert_eq!(latency.mean(), Some(Duration::from_millis(15)));
}
