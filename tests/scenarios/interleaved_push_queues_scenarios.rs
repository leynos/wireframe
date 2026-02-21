//! Scenario tests for interleaved push queue behaviour.

use rstest_bdd_macros::scenario;

use crate::fixtures::interleaved_push_queues::*;

#[scenario(
    path = "tests/features/interleaved_push_queues.feature",
    name = "High-priority frames take precedence when fairness is disabled"
)]
fn strict_priority(interleaved_push_world: InterleavedPushWorld) { let _ = interleaved_push_world; }

#[scenario(
    path = "tests/features/interleaved_push_queues.feature",
    name = "Fairness yields to low-priority after burst threshold"
)]
fn fairness_threshold(interleaved_push_world: InterleavedPushWorld) {
    let _ = interleaved_push_world;
}

#[scenario(
    path = "tests/features/interleaved_push_queues.feature",
    name = "Rate limiting applies symmetrically across both priority levels"
)]
fn rate_limit_symmetry(interleaved_push_world: InterleavedPushWorld) {
    let _ = interleaved_push_world;
}

#[scenario(
    path = "tests/features/interleaved_push_queues.feature",
    name = "All frames are delivered when both queues carry traffic"
)]
fn all_frames_delivered(interleaved_push_world: InterleavedPushWorld) {
    let _ = interleaved_push_world;
}
