//! Unit coverage for workload arithmetic and rejection of unfit workloads.

use super::{ActorWorkload, CancellationMode, measure_actor_drain};

#[test]
fn split_covers_every_frame_across_both_queues() {
    for frames in [1usize, 2, 17, 256] {
        let workload = ActorWorkload {
            frames,
            queue_capacity: 512,
            cancellation: CancellationMode::Disabled,
        };
        assert_eq!(
            workload.high_frames() + workload.low_frames(),
            frames,
            "the two queues must hold every frame for frames={frames}"
        );
    }
}

#[tokio::test]
async fn over_capacity_workload_is_rejected() {
    let error = measure_actor_drain(
        ActorWorkload {
            frames: 3,
            queue_capacity: 1,
            cancellation: CancellationMode::Disabled,
        },
        1,
    )
    .await
    .expect_err("a workload that cannot fit its queues must be rejected");

    assert!(
        error.contains("exceeds total queue capacity"),
        "unexpected rejection reason: {error}"
    );
}

#[tokio::test]
async fn zero_iterations_measures_nothing() {
    let measurement = measure_actor_drain(ActorWorkload::default(), 0)
        .await
        .expect("zero iterations must succeed");

    assert_eq!(measurement.operations, 0);
    assert_eq!(measurement.frames_emitted, 0);
}
