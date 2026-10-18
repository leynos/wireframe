//! What each regime's label advertises, and that it matches what it runs.

use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::{
    AcquisitionMode,
    PoolWorkload,
};

/// Each regime's label names the parameter that actually drives it.
///
/// A contended burst is not driven by [`PoolWorkload::acquisitions`] — the
/// measurement path never reads it — so a contended label that reported
/// `acquisitions_256` would advertise a parameter the run did not honour. The
/// label states `rounds_{CONTENDED_ROUNDS_PER_CONTENDER}` instead, and this
/// test is what stops the two regimes from silently drifting back together.
#[rstest::rstest]
#[case(AcquisitionMode::Uncontended, "acquisitions_256")]
#[case(AcquisitionMode::ContendedFifo, "rounds_8")]
#[case(AcquisitionMode::ContendedRoundRobin, "rounds_8")]
fn labels_name_the_parameter_each_regime_honours(
    #[case] mode: AcquisitionMode,
    #[case] expected_fragment: &str,
) {
    let label = PoolWorkload {
        pool_size: 4,
        mode,
        acquisitions: 256,
        contenders: 16,
    }
    .label();

    assert!(
        label.contains(expected_fragment),
        "a {mode:?} label must report {expected_fragment}: {label}"
    );
    if mode.is_contended() {
        assert!(
            !label.contains("acquisitions_"),
            "a contended label must not advertise the acquisitions field it ignores: {label}"
        );
    }
}

#[test]
fn uncontended_mode_runs_exactly_one_contender() {
    let workload = PoolWorkload {
        pool_size: 4,
        mode: AcquisitionMode::Uncontended,
        acquisitions: 8,
        contenders: 64,
    };
    assert!(
        workload.label().ends_with("contenders_1"),
        "uncontended acquisition must not fan out: {}",
        workload.label()
    );
}
