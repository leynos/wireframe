//! Tests for fragment header accessors and fragment-series sequencing rules.

use rstest::rstest;

use crate::fragment::*;

#[test]
fn fragment_header_exposes_fields() {
    let header = FragmentHeader::new(MessageId::new(9), FragmentIndex::new(2), true);
    assert_eq!(header.message_id(), MessageId::new(9));
    assert_eq!(header.fragment_index(), FragmentIndex::new(2));
    assert!(header.is_last_fragment());
}

#[rstest]
#[case(1)]
#[case(5)]
fn series_accepts_sequential_fragments(#[case] message: u64) {
    let mut series = FragmentSeries::new(MessageId::new(message));
    let first = FragmentHeader::new(MessageId::new(message), FragmentIndex::zero(), false);
    let second = FragmentHeader::new(MessageId::new(message), FragmentIndex::new(1), true);

    assert_eq!(series.accept(first), Ok(FragmentStatus::Incomplete));
    assert_eq!(series.accept(second), Ok(FragmentStatus::Complete));
    assert!(series.is_complete());
}

#[test]
fn series_rejects_other_message() {
    let mut series = FragmentSeries::new(MessageId::new(7));
    let header = FragmentHeader::new(MessageId::new(8), FragmentIndex::zero(), false);
    let err = series
        .accept(header)
        .expect_err("fragment from another message must be rejected");
    assert!(matches!(err, FragmentError::MessageMismatch { .. }));
}

#[test]
fn series_rejects_out_of_order_fragment() {
    let mut series = FragmentSeries::new(MessageId::new(7));
    let header = FragmentHeader::new(MessageId::new(7), FragmentIndex::new(2), false);
    let err = series
        .accept(header)
        .expect_err("out-of-order fragment must be rejected");
    assert!(matches!(err, FragmentError::IndexMismatch { .. }));
}

#[test]
fn series_suppresses_duplicate_fragment() {
    let mut series = FragmentSeries::new(MessageId::new(7));
    let first = FragmentHeader::new(MessageId::new(7), FragmentIndex::zero(), false);
    let duplicate = FragmentHeader::new(MessageId::new(7), FragmentIndex::zero(), false);
    let final_fragment = FragmentHeader::new(MessageId::new(7), FragmentIndex::new(1), true);

    assert_eq!(series.accept(first), Ok(FragmentStatus::Incomplete));
    assert_eq!(series.accept(duplicate), Ok(FragmentStatus::Duplicate));
    assert_eq!(series.accept(final_fragment), Ok(FragmentStatus::Complete));
}

#[test]
fn series_rejects_after_completion() {
    let mut series = FragmentSeries::new(MessageId::new(1));
    let first = FragmentHeader::new(MessageId::new(1), FragmentIndex::zero(), true);
    assert_eq!(series.accept(first), Ok(FragmentStatus::Complete));
    let err = series
        .accept(FragmentHeader::new(
            MessageId::new(1),
            FragmentIndex::new(1),
            true,
        ))
        .expect_err("series must reject fragments after completion");
    assert!(matches!(err, FragmentError::SeriesComplete));
}

#[test]
fn series_reports_duplicate_final_fragment_after_completion() {
    let mut series = FragmentSeries::new(MessageId::new(12));
    let first = FragmentHeader::new(MessageId::new(12), FragmentIndex::zero(), true);
    let duplicate = FragmentHeader::new(MessageId::new(12), FragmentIndex::zero(), true);

    assert_eq!(series.accept(first), Ok(FragmentStatus::Complete));
    assert_eq!(series.accept(duplicate), Ok(FragmentStatus::Duplicate));
    assert!(series.is_complete());
}

#[test]
fn series_detects_index_overflow() {
    let mut series = FragmentSeries::new(MessageId::new(1));
    series.force_next_index_for_tests(FragmentIndex::new(u32::MAX));
    let header = FragmentHeader::new(MessageId::new(1), FragmentIndex::new(u32::MAX), false);
    let err = series
        .accept(header)
        .expect_err("overflow must raise an error");
    assert_eq!(
        err,
        FragmentError::IndexOverflow {
            last: FragmentIndex::new(u32::MAX)
        }
    );
}

#[test]
fn series_accepts_final_fragment_at_max_index() {
    let mut series = FragmentSeries::new(MessageId::new(2));
    series.force_next_index_for_tests(FragmentIndex::new(u32::MAX));
    let header = FragmentHeader::new(MessageId::new(2), FragmentIndex::new(u32::MAX), true);
    assert_eq!(series.accept(header), Ok(FragmentStatus::Complete));
    assert!(series.is_complete());
}
