//! Loom models of the push handle's dead-letter accounting.
//!
//! Producers share one `PushHandle` with the connection actor that drains it.
//! Of everything on that shared path, two things are Loom primitives: the
//! `dlq_drops` counter and the `dlq_last_log` mutex, both touched by
//! `route_to_dlq` when a dropped frame cannot be sent to the dead-letter
//! queue. Those are what these models schedule.
//!
//! The queues themselves are Tokio channels, which Loom does not instrument.
//! The models use them only to put the handle into the state under test, a
//! full queue and a full dead-letter queue, and assert nothing about them: a
//! channel assertion here would pass or fail regardless of any interleaving
//! Loom chose.
#![cfg(loom)]

use std::time::Duration;

use loom::{model, thread};
use rstest::rstest;
use tokio::sync::mpsc;
use wireframe::push::{PushHandle, PushPolicy, PushPriority, PushQueues};

/// Long enough that the interval branch of the drop logging never fires
/// within a model, so the count threshold is the only reset under test.
const NO_INTERVAL_RESET: Duration = Duration::from_secs(3600);

/// Queues, handle and dead-letter receiver, kept alive together.
struct Fixture {
    _queues: PushQueues<u8>,
    handle: PushHandle<u8>,
    _dlq_rx: Option<mpsc::Receiver<u8>>,
}

/// Build a handle whose `priority` queue is full.
///
/// With `dlq_capacity` of `Some(n)` the dead-letter queue exists with room
/// for `n` frames; with `None` there is no dead-letter queue at all.
fn full_queue(priority: PushPriority, dlq_capacity: Option<usize>, log_every_n: usize) -> Fixture {
    let (dlq_tx, dlq_rx) = match dlq_capacity {
        Some(capacity) => {
            let (tx, rx) = mpsc::channel(capacity);
            (Some(tx), Some(rx))
        }
        None => (None, None),
    };
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .dlq(dlq_tx)
        .dlq_log_every_n(log_every_n)
        .dlq_log_interval(NO_INTERVAL_RESET)
        .unlimited()
        .build()
        .expect("queue configuration should be valid");
    handle
        .try_push(0, priority, PushPolicy::ReturnErrorIfFull)
        .expect("the first push fills the empty queue");
    Fixture {
        _queues: queues,
        handle,
        _dlq_rx: dlq_rx,
    }
}

/// Build a handle whose `priority` queue and one-frame dead-letter queue are
/// both full, so every further drop takes `route_to_dlq`'s error branch.
fn full_queue_and_dlq(priority: PushPriority, log_every_n: usize) -> Fixture {
    let fixture = full_queue(priority, Some(1), log_every_n);
    fixture
        .handle
        .try_push(9, priority, PushPolicy::DropIfFull)
        .expect("a drop into a dead-letter queue with room succeeds");
    fixture
}

/// Drop one frame from each of two Loom threads and wait for both.
fn drop_concurrently(handle: &PushHandle<u8>, priority: PushPriority) {
    drop_from_each(handle, priority, &[1, 2]);
}

/// Drop one frame from each of `frames.len()` Loom threads and wait for all.
fn drop_from_each(handle: &PushHandle<u8>, priority: PushPriority, frames: &[u8]) {
    let producers: Vec<_> = frames
        .iter()
        .copied()
        .map(|frame| {
            let producer = handle.clone();
            thread::spawn(move || {
                producer
                    .try_push(frame, priority, PushPolicy::DropIfFull)
                    .expect("a drop is not an error under DropIfFull");
            })
        })
        .collect();
    for producer in producers {
        producer.join().expect("producer thread panicked");
    }
}

#[rstest]
#[case::high(PushPriority::High)]
#[case::low(PushPriority::Low)]
fn concurrent_drops_are_all_counted(#[case] priority: PushPriority) {
    // Threshold three, two drops: the reset cannot fire, so the counter must
    // read exactly two on every interleaving. This is the increment rule on
    // its own, observed before any reset can hide it.
    model(move || {
        let fixture = full_queue_and_dlq(priority, 3);
        let probe = fixture.handle.probe();
        drop_concurrently(&fixture.handle, priority);
        assert_eq!(
            probe.dlq_drop_count(),
            2,
            "two failed dead-letter sends must count two, whatever the interleaving"
        );
    });
}

#[rstest]
#[case::high(PushPriority::High)]
#[case::low(PushPriority::Low)]
fn the_counter_resets_at_the_logging_threshold(#[case] priority: PushPriority) {
    // Threshold two, two drops: whichever producer increments second reaches
    // the threshold, logs and resets, so the counter must end at zero.
    model(move || {
        let fixture = full_queue_and_dlq(priority, 2);
        let probe = fixture.handle.probe();
        drop_concurrently(&fixture.handle, priority);
        assert_eq!(
            probe.dlq_drop_count(),
            0,
            "reaching the logging threshold must reset the counter"
        );
    });
}

#[rstest]
#[case::high(PushPriority::High)]
#[case::low(PushPriority::Low)]
fn every_drop_is_reported_or_still_counted(#[case] priority: PushPriority) {
    // Three producers, threshold two: at least one producer reaches the
    // threshold and reports, while the others may increment around its
    // reset. Every failed dead-letter send must end up either in a report
    // or still in the counter, never in both and never in neither, so the
    // two together equal the three drops on every interleaving. A reset that
    // reports one value and clears another loses or double-counts drops.
    model(move || {
        let fixture = full_queue_and_dlq(priority, 2);
        let probe = fixture.handle.probe();
        drop_from_each(&fixture.handle, priority, &[1, 2, 3]);
        let reported = probe.dlq_reported_count();
        let remaining = probe.dlq_drop_count();
        assert_eq!(
            reported + remaining,
            3,
            "reported ({reported}) plus remaining ({remaining}) must equal the three drops"
        );
    });
}

#[test]
fn drops_into_a_dead_letter_queue_with_room_count_nothing() {
    // The narrowness half of the increment rule: only a failed dead-letter
    // send counts. Two drops land in a queue with room for both.
    model(|| {
        let fixture = full_queue(PushPriority::High, Some(2), 3);
        let probe = fixture.handle.probe();
        drop_concurrently(&fixture.handle, PushPriority::High);
        assert_eq!(
            probe.dlq_drop_count(),
            0,
            "a drop the dead-letter queue accepted is not a lost frame"
        );
    });
}

#[test]
fn drops_without_a_dead_letter_queue_count_nothing() {
    // Without a dead-letter queue there is nothing to fail to send to, so
    // nothing is counted.
    model(|| {
        let fixture = full_queue(PushPriority::High, None, 3);
        let probe = fixture.handle.probe();
        drop_concurrently(&fixture.handle, PushPriority::High);
        assert_eq!(
            probe.dlq_drop_count(),
            0,
            "the counter must stay at zero when no dead-letter queue is configured"
        );
    });
}
