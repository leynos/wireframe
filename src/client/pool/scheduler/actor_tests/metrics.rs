//! Metric coverage for the scheduler's abnormal-termination paths.
//!
//! Both tests assert on the same unlabelled scheduler-failure counter, so they
//! and the snapshot helper that reads it live together.

use super::*;

/// Count how many times the scheduler-failure counter was recorded.
///
/// The counter is deliberately unlabelled, so the name alone identifies it.
fn scheduler_failure_count(snapshotter: &metrics_util::debugging::Snapshotter) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .iter()
        .filter(|(key, ..)| key.key().name() == crate::metrics::POOL_SCHEDULER_FAILURES)
        .map(|(_, _, _, value)| match value {
            metrics_util::debugging::DebugValue::Counter(count) => *count,
            _ => 0,
        })
        .sum()
}

/// A permit-race failure is the scheduler's other abnormal-termination path,
/// and it must be observable as well as recorded in the lifecycle.
#[test]
fn a_permit_race_error_records_the_scheduler_failure_metric() {
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);
    harness.hold_only_permit();
    let _queued = harness.acquire(1);

    ::metrics::with_local_recorder(&recorder, || {
        harness
            .task
            .handle_permit_outcome(&harness.core, Some(Err(ClientError::scheduler_failed())));
    });

    assert_eq!(
        harness.status.lifecycle(),
        SchedulerLifecycle::Failed,
        "scheduler marked failed"
    );
    assert_eq!(scheduler_failure_count(&snapshotter), 1, "single failure");
}

/// A task that unwinds publishes its failure through `Drop`, which is the only
/// place the panic path can record the metric.
#[test]
fn a_panicking_scheduler_task_records_the_scheduler_failure_metric() {
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build a current-thread runtime");
        let (sender, receiver) = mpsc::unbounded_channel();
        let core = {
            // A slot's backing pool builds a time interval eagerly, so it needs
            // the runtime's reactor even though this test drives it to panic.
            let _guard = runtime.enter();
            Arc::new(PoolCore::new(
                vec![test_pool_slot(1, Duration::from_secs(30))].into_boxed_slice(),
            ))
        };
        let status = Arc::new(SchedulerStatus::new(Arc::new(
            PoolTaskChurnCounts::default(),
        )));
        let task = SchedulerTask::new(
            Arc::clone(&core),
            Arc::clone(&status),
            receiver,
            WaiterAdmission {
                fairness_policy: PoolFairnessPolicy::Fifo,
                max_waiters: 8,
            },
        );
        let join = {
            let _guard = runtime.enter();
            tokio::spawn(task.run())
        };

        sender
            .send(SchedulerCommand::PanicForTest)
            .expect("the mailbox must accept the panic injection");
        let joined = runtime.block_on(join);

        assert!(joined.is_err(), "panic must terminate the task");
        assert_eq!(status.lifecycle(), SchedulerLifecycle::Failed, "has failed");
    });

    assert_eq!(scheduler_failure_count(&snapshotter), 1, "panic failure");
}
