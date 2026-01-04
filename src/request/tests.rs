//! Unit tests for [`RequestParts`] and correlation handling.

use rstest::rstest;

use super::*;

#[rstest]
fn new_stores_all_fields() {
    let parts = RequestParts::new(42, Some(100), vec![0x01, 0x02]);
    assert_eq!(parts.id(), 42);
    assert_eq!(parts.correlation_id(), Some(100));
    assert_eq!(parts.metadata(), &[0x01, 0x02]);
}

#[rstest]
fn metadata_returns_reference() {
    let parts = RequestParts::new(1, None, vec![0xab, 0xcd, 0xef]);
    let meta = parts.metadata();
    assert_eq!(meta.len(), 3);
    assert_eq!(meta.first(), Some(&0xab));
}

#[rstest]
fn metadata_mut_allows_modification() {
    let mut parts = RequestParts::new(1, None, vec![0x01]);
    parts.metadata_mut().push(0x02);
    assert_eq!(parts.metadata(), &[0x01, 0x02]);
}

#[rstest]
fn into_metadata_transfers_ownership() {
    let parts = RequestParts::new(1, None, vec![7, 8, 9]);
    let owned = parts.into_metadata();
    assert_eq!(owned, vec![7, 8, 9]);
}

#[rstest]
fn clone_produces_equal_instance() {
    let parts = RequestParts::new(42, Some(100), vec![0x01, 0x02]);
    let cloned = parts.clone();
    assert_eq!(parts, cloned);
}

#[rstest]
#[case(RequestParts::new(1, None, vec![]), Some(42), Some(42))]
#[case(RequestParts::new(1, Some(7), vec![]), None, Some(7))]
#[case(RequestParts::new(1, None, vec![]), None, None)]
#[case(RequestParts::new(1, Some(7), vec![]), Some(8), Some(8))]
fn inherit_correlation_variants(
    #[case] start: RequestParts,
    #[case] source: Option<u64>,
    #[case] expected: Option<u64>,
) {
    let got = start.inherit_correlation(source);
    assert_eq!(got.correlation_id(), expected);
}

#[rstest]
fn empty_metadata_is_valid() {
    let parts = RequestParts::new(1, None, vec![]);
    assert!(parts.metadata().is_empty());
}
