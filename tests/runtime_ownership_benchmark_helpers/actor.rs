//! V-O3 — the actor drains exactly the queued frames, and the cancellation modes
//! are distinguishable.
//!
//! A frame count alone cannot tell the three cancellation regimes apart: two of
//! them emit every frame. Each case below therefore asserts the regime's
//! distinguishing observable beside its frame count, so a change that collapsed
//! one regime onto another would fail rather than pass under a shared label.

use rstest::rstest;
use wireframe_testing::runtime_ownership_benchmarks::{
    ActorWorkload,
    CancellationMode,
    DEFAULT_ACTOR_QUEUE_CAPACITY,
    DEFAULT_FRAME_COUNT,
    measure_actor_drain,
};

use super::harness::{ITERATIONS, block_on};

#[rstest]
#[case::single(1)]
#[case::awkward_odd(17)]
#[case::bulk(256)]
fn actor_drains_every_queued_frame_with_cancellation_disabled(#[case] frames: usize) {
    let measurement = block_on(measure_actor_drain(
        ActorWorkload {
            frames,
            queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
            cancellation: CancellationMode::Disabled,
        },
        ITERATIONS,
    ))
    .expect("actor drain measurement failed");

    assert_eq!(
        measurement.frames_emitted, frames,
        "the loop must emit every preloaded frame, across both priority lanes"
    );
    assert!(
        !measurement.cancelled_during_drain,
        "the disabled regime must not record an observed cancellation"
    );
}

#[test]
fn actor_emits_nothing_when_cancelled_before_it_runs() {
    let measurement = block_on(measure_actor_drain(
        ActorWorkload {
            frames: 256,
            queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
            cancellation: CancellationMode::CancelledBeforeRun,
        },
        ITERATIONS,
    ))
    .expect("cancelled actor drain measurement failed");

    assert_eq!(
        measurement.frames_emitted, 0,
        "a token cancelled before the run must abort the loop before it drains"
    );
}

#[test]
fn actor_still_emits_every_frame_when_cancelled_mid_drain() {
    let frames = 256;
    let measurement = block_on(measure_actor_drain(
        ActorWorkload {
            frames,
            queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
            cancellation: CancellationMode::CancelledDuringDrain,
        },
        ITERATIONS,
    ))
    .expect("mid-drain cancelled actor measurement failed");

    assert!(
        measurement.cancelled_during_drain,
        "the output hook must have observed the cancellation, otherwise this case is \
         indistinguishable from the disabled regime"
    );
    assert_eq!(
        measurement.frames_emitted, frames,
        "observing shutdown mid-drain must not abandon frames already queued"
    );
}

#[test]
fn default_actor_workload_fits_its_queues() {
    let workload = ActorWorkload {
        frames: DEFAULT_FRAME_COUNT,
        queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
        cancellation: CancellationMode::Disabled,
    };
    // The helper rejects an over-capacity workload, so a successful measurement
    // is itself the assertion that the default fits without back-pressure.
    block_on(measure_actor_drain(workload, ITERATIONS))
        .expect("the default actor workload must fit its queues");
}
