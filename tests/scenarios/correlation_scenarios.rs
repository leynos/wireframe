//! Scenario tests for `correlation_id` feature.

use rstest_bdd_macros::scenario;

use crate::fixtures::correlation::*;

#[scenario(
    path = "tests/features/correlation_id.feature",
    name = "Streamed frames reuse the request correlation id"
)]
fn streamed_frames_correlation(correlation_world: CorrelationWorld) { let _ = correlation_world; }

#[scenario(
    path = "tests/features/correlation_id.feature",
    name = "Multi-packet responses reuse the request correlation id"
)]
fn multi_packet_correlation(correlation_world: CorrelationWorld) { let _ = correlation_world; }

#[scenario(
    path = "tests/features/correlation_id.feature",
    name = "Multi-packet responses clear correlation ids without a request id"
)]
fn no_correlation(correlation_world: CorrelationWorld) { let _ = correlation_world; }
