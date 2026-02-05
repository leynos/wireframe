#![cfg_attr(loom, allow(missing_docs))]
#![cfg(not(loom))]
//! Tests for `RequestParts` accessors and correlation inheritance.

use wireframe::request::RequestParts;

#[test]
fn request_parts_round_trip() {
    let parts = RequestParts::new(5, Some(99), vec![1, 2, 3]);
    let id = parts.id();
    let correlation_id = parts.correlation_id();
    let metadata = parts.metadata().to_vec();
    assert_eq!(id, 5);
    assert_eq!(correlation_id, Some(99));
    assert_eq!(metadata, vec![1, 2, 3]);
}

#[test]
fn request_parts_into_metadata_consumes() {
    let parts = RequestParts::new(1, None, vec![10, 20, 30]);
    let owned = parts.into_metadata();
    assert_eq!(owned, vec![10, 20, 30]);
}

#[test]
fn request_parts_metadata_mut_modification() {
    let mut parts = RequestParts::new(1, None, vec![0x01]);
    parts.metadata_mut().push(0x02);
    parts.metadata_mut().push(0x03);
    assert_eq!(parts.metadata(), &[0x01, 0x02, 0x03]);
}

// NOTE: Correlation inheritance is tested via:
// - Unit tests in `src/request.rs::inherit_correlation_variants`
// - Behavioural tests in `tests/features/request_parts.feature`

#[test]
fn request_parts_clone_equality() {
    let original = RequestParts::new(42, Some(123), vec![0xab, 0xcd]);
    let cloned = original.clone();
    assert_eq!(original, cloned);
}

#[test]
fn request_parts_debug_format() {
    let parts = RequestParts::new(1, Some(2), vec![3]);
    let debug = format!("{parts:?}");
    assert!(debug.contains("RequestParts"));
    assert!(debug.contains("id: 1"));
    assert!(debug.contains("correlation_id: Some(2)"));
}
