//! Concurrency tests for push queues using loom.
//!
//! These tests exercise the `PushHandle` shared state without Tokio. `loom`
//! explores interleavings to ensure DLQ accounting and queue-full errors
//! remain deterministic under concurrent producers.
#![cfg(all(feature = "advanced-tests", loom))]

use loom::{model, thread};
use rstest::rstest;
use tokio::sync::mpsc;
use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};

#[rstest]
#[case(PushPriority::High)]
#[case(PushPriority::Low)]
fn concurrent_drops_reset_dlq_counter(#[case] priority: PushPriority) {
    model(move || {
        let (dlq_tx, mut dlq_rx) = mpsc::channel(4);
        let (queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(1)
            .low_capacity(1)
            .dlq(Some(dlq_tx))
            .dlq_log_every_n(2)
            .unlimited()
            .build()
            .expect("failed to build PushQueues");
        let _queues = queues;

        handle
            .try_push(0, priority, PushPolicy::ReturnErrorIfFull)
            .expect("initial push should succeed");

        let probe = handle.probe();
        let h1 = handle.clone();
        let h2 = handle.clone();

        let t1_priority = priority;
        let t2_priority = priority;

        let t1 = thread::spawn(move || {
            h1.try_push(1, t1_priority, PushPolicy::WarnAndDropIfFull)
                .expect("first drop should succeed");
        });
        let t2 = thread::spawn(move || {
            h2.try_push(2, t2_priority, PushPolicy::WarnAndDropIfFull)
                .expect("second drop should succeed");
        });

        t1.join().expect("first drop thread panicked");
        t2.join().expect("second drop thread panicked");

        assert_eq!(
            probe.dlq_drop_count(),
            0,
            "counter should reset after reaching the logging threshold"
        );

        let mut drops = Vec::new();
        while let Ok(frame) = dlq_rx.try_recv() {
            drops.push(frame);
        }
        drops.sort_unstable();
        assert_eq!(drops, vec![1, 2]);
    });
}

#[test]
fn concurrent_queue_full_errors_are_reported() {
    model(|| {
        let (queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(1)
            .low_capacity(1)
            .unlimited()
            .build()
            .expect("failed to build PushQueues");
        let _queues = queues;

        handle
            .try_push(0, PushPriority::High, PushPolicy::ReturnErrorIfFull)
            .expect("initial push should succeed");

        let h1 = handle.clone();
        let h2 = handle.clone();
        let h3 = handle.clone();

        let t1 = thread::spawn(move || {
            let res = h1.try_push(1, PushPriority::High, PushPolicy::ReturnErrorIfFull);
            assert!(
                matches!(res, Err(PushError::QueueFull)),
                "expected queue full error for first producer"
            );
        });
        let t2 = thread::spawn(move || {
            let res = h2.try_push(2, PushPriority::High, PushPolicy::ReturnErrorIfFull);
            assert!(
                matches!(res, Err(PushError::QueueFull)),
                "expected queue full error for second producer"
            );
        });
        let t3 = thread::spawn(move || {
            let res = h3.try_push(3, PushPriority::High, PushPolicy::ReturnErrorIfFull);
            assert!(
                matches!(res, Err(PushError::QueueFull)),
                "expected queue full error for third producer"
            );
        });

        t1.join().expect("first producer thread panicked");
        t2.join().expect("second producer thread panicked");
        t3.join().expect("third producer thread panicked");
    });
}

#[test]
fn dlq_probe_ignores_absent_channel() {
    model(|| {
        let (queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(1)
            .low_capacity(1)
            .dlq(None)
            .dlq_log_every_n(2)
            .unlimited()
            .build()
            .expect("failed to build PushQueues");
        let _queues = queues;

        handle
            .try_push(0, PushPriority::High, PushPolicy::ReturnErrorIfFull)
            .expect("initial push should succeed");

        let probe = handle.probe();

        handle
            .try_push(1, PushPriority::High, PushPolicy::WarnAndDropIfFull)
            .expect("drop should succeed even without a DLQ");

        assert_eq!(
            probe.dlq_drop_count(),
            0,
            "counter remains zero when DLQ is disabled"
        );
    });
}

#[test]
fn dlq_probe_reports_zero_when_dlq_idle() {
    model(|| {
        let (dlq_tx, mut dlq_rx) = mpsc::channel(2);
        let (queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(1)
            .low_capacity(1)
            .dlq(Some(dlq_tx))
            .dlq_log_every_n(2)
            .unlimited()
            .build()
            .expect("failed to build PushQueues");
        let _queues = queues;

        let probe = handle.probe();

        assert_eq!(
            probe.dlq_drop_count(),
            0,
            "counter remains zero before any drops"
        );
        assert!(
            dlq_rx.try_recv().is_err(),
            "DLQ should be empty before any drops"
        );
    });
}
