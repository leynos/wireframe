//! Tests for fixed slot storage, rotation-aware acquisition, and metrics.
//!
//! The suite is split by concern so each file stays reviewable:
//! `support` holds the shared cores and fixtures, `sequence_tests` covers slot
//! identity and lifecycle, `permit_race` covers asynchronous selection and
//! cancellation, and `metric_tests` covers the labels the real acquisition
//! wrappers record.

mod metric_tests;
mod permit_race;
mod sequence_tests;
mod support;
mod transition_model;
