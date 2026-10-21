//! Tests for fixed slot storage, rotation-aware acquisition, and metrics.
//!
//! The suite is split by concern so each file stays reviewable:
//! `support` holds the shared cores and fixtures, `sequence_tests` covers slot
//! identity and lifecycle, and `metric_tests` covers the labels the real
//! acquisition wrappers record.

mod metric_tests;
mod sequence_tests;
mod support;
