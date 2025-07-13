//! Tests for push queue routing and limits.
//!
//! They cover priority ordering, policy behaviour, and closed queue errors.

use rstest::rstest;
use tokio::time::{self, Duration};
use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};

/// Frames are delivered to queues matching their push priority.
#[tokio::test]
async fn frames_routed_to_correct_priority_queues() {
    let (mut queues, handle) = PushQueues::bounded(1, 1);

    handle.push_low_priority(1u8).await.unwrap();
    handle.push_high_priority(2u8).await.unwrap();

    let (prio1, frame1) = queues.recv().await.unwrap();
    let (prio2, frame2) = queues.recv().await.unwrap();

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
    let (mut queues, handle) = PushQueues::bounded(1, 1);

    handle.push_high_priority(1u8).await.unwrap();
    let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    assert!(result.is_err());

    // drain queue to allow new push
    let _ = queues.recv().await;
    handle.push_high_priority(3u8).await.unwrap();
    let (_, last) = queues.recv().await.unwrap();
    assert_eq!(last, 3);
}

/// Push attempts return `Closed` when all queues have been shut down.
#[tokio::test]
async fn push_queues_error_on_closed() {
    let (queues, handle) = PushQueues::bounded(1, 1);

    let mut queues = queues;
    queues.close();
    let res = handle.push_high_priority(42u8).await;
    assert!(matches!(res, Err(PushError::Closed)));

    let res = handle.push_low_priority(24u8).await;
    assert!(matches!(res, Err(PushError::Closed)));
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
    let (mut queues, handle) = PushQueues::bounded_with_rate(2, 2, Some(1)).unwrap();

    match priority {
        PushPriority::High => handle.push_high_priority(1u8).await.unwrap(),
        PushPriority::Low => handle.push_low_priority(1u8).await.unwrap(),
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
        PushPriority::High => handle.push_high_priority(3u8).await.unwrap(),
        PushPriority::Low => handle.push_low_priority(3u8).await.unwrap(),
    }

    let (_, first) = queues.recv().await.unwrap();
    let (_, second) = queues.recv().await.unwrap();
    assert_eq!((first, second), (1, 3));
}

/// Exceeding the rate limit succeeds after the window has passed.
#[tokio::test]
async fn rate_limiter_allows_after_wait() {
    time::pause();
    let (mut queues, handle) = PushQueues::bounded_with_rate(2, 2, Some(1)).unwrap();
    handle.push_high_priority(1u8).await.unwrap();
    time::advance(Duration::from_secs(1)).await;
    handle.push_high_priority(2u8).await.unwrap();

    let (_, a) = queues.recv().await.unwrap();
    let (_, b) = queues.recv().await.unwrap();
    assert_eq!((a, b), (1, 2));
}

/// The limiter counts pushes from all priority queues.
/// The token bucket is shared, so pushes from one priority reduce
/// the allowance for the other.
#[tokio::test]
async fn rate_limiter_shared_across_priorities() {
    time::pause();
    let (mut queues, handle) = PushQueues::bounded_with_rate(2, 2, Some(1)).unwrap();
    handle.push_high_priority(1u8).await.unwrap();

    let attempt = time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await;
    assert!(attempt.is_err(), "second push should block across queues");

    time::advance(Duration::from_secs(1)).await;
    handle.push_low_priority(2u8).await.unwrap();

    let (prio1, frame1) = queues.recv().await.unwrap();
    let (prio2, frame2) = queues.recv().await.unwrap();
    assert_eq!(prio1, PushPriority::High);
    assert_eq!(frame1, 1);
    assert_eq!(prio2, PushPriority::Low);
    assert_eq!(frame2, 2);
}

/// Unlimited queues never block pushes.
#[tokio::test]
async fn unlimited_queues_do_not_block() {
    time::pause();
    let (mut queues, handle) = PushQueues::bounded_no_rate_limit(1, 1);
    handle.push_high_priority(1u8).await.unwrap();
    let res = time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await;
    assert!(res.is_ok(), "pushes should not block when unlimited");

    let (_, a) = queues.recv().await.unwrap();
    let (_, b) = queues.recv().await.unwrap();
    assert_eq!((a, b), (1, 2));
}

/// A burst up to capacity succeeds and further pushes are blocked.
/// The maximum burst size equals the configured `capacity` parameter.
#[tokio::test]
async fn rate_limiter_allows_burst_within_capacity_and_blocks_excess() {
    time::pause();
    let (mut queues, handle) = PushQueues::bounded_with_rate(4, 4, Some(3)).unwrap();

    for i in 0u8..3 {
        handle.push_high_priority(i).await.unwrap();
    }

    let res = time::timeout(Duration::from_millis(10), handle.push_high_priority(99)).await;
    assert!(
        res.is_err(),
        "Push exceeding burst capacity should be rate limited"
    );

    time::advance(Duration::from_secs(1)).await;
    handle.push_high_priority(100).await.unwrap();

    for expected in [0u8, 1u8, 2u8, 100u8] {
        let (_, frame) = queues.recv().await.unwrap();
        assert_eq!(frame, expected);
    }
}
