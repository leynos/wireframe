//! Proves the copied-byte recorders compile in both feature configurations.
//!
//! The workspace's own tests cannot make the disabled half of this check:
//! `wireframe_testing` enables `wireframe/benchmark-counters` for every test in
//! the repository, so a recorder that became unconditional would still look
//! correct there. This consumer depends on `wireframe` alone and enables the
//! feature only when a check says so.
//!
//! The pair matters. The disabled check fails if a recorder stops being inert,
//! which would make production builds pay for counters nobody reads. The
//! enabled check is its control: it fails if the recorders stop counting, which
//! would make the disabled check vacuous because nothing was ever counted in
//! either configuration.

use wireframe::benchmark_counters;

/// Bytes each recorder is asked to count.
const RECORDED_BYTES: usize = 16;

/// Call every recorder once and return what the counters observed.
fn record_every_seam() -> benchmark_counters::CopiedBytes {
    benchmark_counters::reset();
    benchmark_counters::record_serialized(RECORDED_BYTES);
    benchmark_counters::record_handoff(RECORDED_BYTES);
    benchmark_counters::record_framed(RECORDED_BYTES);
    benchmark_counters::record_deserialized(RECORDED_BYTES);
    benchmark_counters::snapshot()
}

/// Without the feature, every recorder must compile and count nothing.
#[cfg(not(feature = "benchmark-counters"))]
#[test]
fn recorders_are_inert_without_the_feature() {
    let copied = record_every_seam();
    assert_eq!(
        copied,
        benchmark_counters::CopiedBytes::default(),
        "a build without benchmark-counters must report no copied bytes: {copied:?}"
    );
}

/// With the feature, the same calls must accumulate their bytes.
#[cfg(feature = "benchmark-counters")]
#[test]
fn recorders_accumulate_with_the_feature() {
    let copied = record_every_seam();
    let expected = 4 * RECORDED_BYTES as u64;
    assert_eq!(
        copied.total(),
        expected,
        "each of the four seams should count {RECORDED_BYTES} bytes: {copied:?}"
    );
}
