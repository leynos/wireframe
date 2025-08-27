//! Tests for push queue routing and limits.
//!
//! They cover priority ordering, policy behaviour, and closed queue errors.

use rstest::rstest;
use tokio::time::{self, Duration};
use wireframe::push::{
    MAX_QUEUE_CAPACITY,
    PushConfigError,
    PushError,
    PushPolicy,
    PushPriority,
    PushQueues,
    PushQueuesBuilder,
};
use wireframe_testing::{push_expect, recv_expect};

// Pause Tokio's clock and build queues with the provided capacity and rate
// configuration. The helper ensures a consistent setup for timing-sensitive
// push tests.
fn setup_timed_push_test(
    capacity: (usize, usize),
    rate_config: impl FnOnce(PushQueuesBuilder<u8>) -> PushQueuesBuilder<u8>,
) -> (PushQueues<u8>, wireframe::push::PushHandle<u8>) {
    time::pause();
    let (high, low) = capacity;
    rate_config(PushQueues::builder().capacity(high, low))
        .build()
        .expect("queue creation failed")
}

/// Frames are delivered to queues matching their push priority.
#[tokio::test]
async fn frames_routed_to_correct_priority_queues() {
    let (mut queues, handle) = PushQueues::builder()
        .capacity(1, 1)
        .build()
        .expect("queue creation failed");

    push_expect!(handle.push_low_priority(1u8));
    push_expect!(handle.push_high_priority(2u8));

    let (prio1, frame1) = recv_expect!(queues.recv());
    let (prio2, frame2) = recv_expect!(queues.recv());

    assert_eq!(prio1, PushPriority::High);
    assert_eq!(frame1, 2);
    assert_eq!(prio2, PushPriority::Low);
    assert_eq!(frame2, 1);
}

/// `try_push` honours the selected queue policy when full.
///
/// Using [`PushPolicy::ReturnErrorIfFull`] causes `try_push` to
/// return `PushError::Full` once the queue is at capacity.
#[tokio::test]
async fn try_push_respects_policy() {
    let (mut queues, handle) = PushQueues::builder()
        .capacity(1, 1)
        .build()
        .expect("queue creation failed");

    push_expect!(handle.push_high_priority(1u8));
    let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    assert!(result.is_err());

    // drain queue to allow new push
    let _ = queues.recv().await;
    push_expect!(handle.push_high_priority(3u8));
    let (_, last) = recv_expect!(queues.recv());
    assert_eq!(last, 3);
}

/// Push attempts return `Closed` when all queues have been shut down.
#[tokio::test]
async fn push_queues_error_on_closed() {
    let (queues, handle) = PushQueues::builder()
        .capacity(1, 1)
        .build()
        .expect("queue creation failed");

    let mut queues = queues;
    queues.close();
    let res = handle.push_high_priority(42u8).await;
    assert!(matches!(res, Err(PushError::Closed)));

    let res = handle.push_low_priority(24u8).await;
    assert!(matches!(res, Err(PushError::Closed)));
}

/// Low-priority frames are eventually yielded after a burst of high-priority
/// traffic.
#[tokio::test]
async fn low_priority_yielded_after_high_burst() {
    const BURST: usize = 8; // keep in sync with HIGH_PRIORITY_BURST_LIMIT
    const BURST_U8: u8 = 8;
    let (mut queues, handle) = PushQueues::builder()
        .capacity(BURST + 2, BURST + 2)
        .build()
        .expect("queue creation failed");

    push_expect!(handle.push_low_priority(0u8));
    for i in 0..=BURST_U8 {
        push_expect!(handle.push_high_priority(i + 1));
    }

    for _ in 0..BURST {
        let (priority, _) = recv_expect!(queues.recv());
        assert_eq!(priority, PushPriority::High);
    }
    let (priority, _) = recv_expect!(queues.recv());
    assert_eq!(priority, PushPriority::Low);
}

/// A push beyond the configured rate is blocked.
/// Time is paused using [`tokio::time::pause`], so the test runs in a
/// virtual-time context.
#[rstest]
#[case::high(PushPriority::High)]
#[case::low(PushPriority::Low)]
#[tokio::test]
async fn rate_limiter_blocks_when_exceeded(#[case] priority: PushPriority) {
    time::pause();
    let (mut queues, handle) = PushQueues::builder()
        .capacity(2, 2)
        .rate(Some(1))
        .build()
        .expect("queue creation failed");

    match priority {
        PushPriority::High => push_expect!(handle.push_high_priority(1u8)),
        PushPriority::Low => push_expect!(handle.push_low_priority(1u8)),
    }

    let attempt = match priority {
        PushPriority::High => {
            time::timeout(Duration::from_millis(10), handle.push_high_priority(2u8)).await
        }
        PushPriority::Low => {
            time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await
        }
    };

    assert!(attempt.is_err(), "second push should block");

    time::advance(Duration::from_secs(1)).await;
    match priority {
        PushPriority::High => push_expect!(handle.push_high_priority(3u8)),
        PushPriority::Low => push_expect!(handle.push_low_priority(3u8)),
    }

    let (_, first) = recv_expect!(queues.recv());
    let (_, second) = recv_expect!(queues.recv());
    assert_eq!((first, second), (1, 3));
}

/// Exceeding the rate limit succeeds after the window has passed.
#[tokio::test]
async fn rate_limiter_allows_after_wait() {
    let (mut queues, handle) = setup_timed_push_test((2, 2), |b| b.rate(Some(1)));
    push_expect!(handle.push_high_priority(1u8));
    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_high_priority(2u8));

    let (_, a) = recv_expect!(queues.recv());
    let (_, b) = recv_expect!(queues.recv());
    assert_eq!((a, b), (1, 2));
}

/// The limiter counts pushes from all priority queues.
/// The token bucket is shared, so pushes from one priority reduce
/// the allowance for the other.
#[tokio::test]
async fn rate_limiter_shared_across_priorities() {
    time::pause();
    let (mut queues, handle) = PushQueues::builder()
        .capacity(2, 2)
        .rate(Some(1))
        .build()
        .expect("queue creation failed");
    push_expect!(handle.push_high_priority(1u8));

    let attempt = time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await;
    assert!(attempt.is_err(), "second push should block across queues");

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_low_priority(2u8));

    let (prio1, frame1) = recv_expect!(queues.recv());
    let (prio2, frame2) = recv_expect!(queues.recv());
    assert_eq!(prio1, PushPriority::High);
    assert_eq!(frame1, 1);
    assert_eq!(prio2, PushPriority::Low);
    assert_eq!(frame2, 2);
}

/// Unlimited queues never block pushes.
#[tokio::test]
async fn unlimited_queues_do_not_block() {
    let (mut queues, handle) = setup_timed_push_test((1, 1), PushQueuesBuilder::no_rate_limit);
    push_expect!(handle.push_high_priority(1u8));
    let res = time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await;
    assert!(res.is_ok(), "pushes should not block when unlimited");

    let (_, a) = recv_expect!(queues.recv());
    let (_, b) = recv_expect!(queues.recv());
    assert_eq!((a, b), (1, 2));
}

/// A burst up to capacity succeeds and further pushes are blocked.
/// The maximum burst size equals the configured `capacity` parameter.
#[tokio::test]
async fn rate_limiter_allows_burst_within_capacity_and_blocks_excess() {
    time::pause();
    let (mut queues, handle) = PushQueues::builder()
        .capacity(4, 4)
        .rate(Some(3))
        .build()
        .expect("queue creation failed");

    for i in 0u8..3 {
        push_expect!(handle.push_high_priority(i));
    }

    let res = time::timeout(Duration::from_millis(10), handle.push_high_priority(99)).await;
    assert!(
        res.is_err(),
        "Push exceeding burst capacity should be rate limited"
    );

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_high_priority(100));

    for expected in [0u8, 1u8, 2u8, 100u8] {
        let (_, frame) = recv_expect!(queues.recv());
        assert_eq!(frame, expected);
    }
}

/// Builder rejects zero or oversized capacities.
#[tokio::test]
async fn invalid_capacities_error() {
    let err = PushQueues::<u8>::builder().capacity(0, 1).build();
    assert!(matches!(
        err,
        Err(PushConfigError::InvalidCapacity {
            queue: PushPriority::High,
            ..
        })
    ));

    let err = PushQueues::<u8>::builder()
        .capacity(MAX_QUEUE_CAPACITY + 1, 1)
        .build();
    assert!(matches!(
        err,
        Err(PushConfigError::InvalidCapacity {
            queue: PushPriority::High,
            ..
        })
    ));
}
