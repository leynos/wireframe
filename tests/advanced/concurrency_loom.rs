#![cfg(all(feature = "advanced-tests", loom))]
//! Concurrency tests for push queues using loom.
//!
//! These tests exercise the `PushHandle` shared state without Tokio. `loom`
//! explores interleavings to ensure DLQ accounting and queue-full errors remain
//! deterministic under concurrent producers.

use loom::{model, thread};
use tokio::sync::mpsc;
use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};

#[test]
fn concurrent_drops_reset_dlq_counter() {
    model(|| {
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
            .try_push(0, PushPriority::High, PushPolicy::ReturnErrorIfFull)
            .expect("initial push should succeed");

        let probe = handle.probe();
        let h1 = handle.clone();
        let h2 = handle.clone();

        let t1 = thread::spawn(move || {
            h1.try_push(1, PushPriority::High, PushPolicy::WarnAndDropIfFull)
                .expect("first drop should succeed");
        });
        let t2 = thread::spawn(move || {
            h2.try_push(2, PushPriority::High, PushPolicy::WarnAndDropIfFull)
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
        drops.sort();
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

        t1.join().expect("first producer thread panicked");
        t2.join().expect("second producer thread panicked");
    });
}
