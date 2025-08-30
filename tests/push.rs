//! Tests for push queue routing and limits.
//!
//! They cover priority ordering, policy behaviour, and closed queue errors.

use rstest::{fixture, rstest};
use tokio::time::{self, Duration};
use wireframe::push::{
    MAX_PUSH_RATE,
    PushConfigError,
    PushError,
    PushHandle,
    PushPolicy,
    PushPriority,
    PushQueues,
};
use wireframe_testing::{push_expect, recv_expect};

#[fixture]
fn queues() -> (PushQueues<u8>, PushHandle<u8>) {
    PushQueues::<u8>::builder()
        .high_capacity(2)
        .low_capacity(2)
        .rate(Some(1))
        .build()
        .expect("failed to build PushQueues")
}

#[fixture]
fn small_queues() -> (PushQueues<u8>, PushHandle<u8>) {
    PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("failed to build PushQueues")
}

/// Builder rejects rates outside the supported range.
#[rstest]
#[case::zero(0)]
#[case::too_high(MAX_PUSH_RATE + 1)]
fn builder_rejects_invalid_rate(#[case] rate: usize) {
    let result = PushQueues::<u8>::builder().rate(Some(rate)).build();
    assert!(matches!(result, Err(PushConfigError::InvalidRate(r)) if r == rate));
}

/// Setting the rate to `None` disables throttling without error.
#[test]
fn builder_accepts_none_rate() {
    let result = PushQueues::<u8>::builder().rate(None).build();
    assert!(result.is_ok(), "builder should accept None rate");
}

#[test]
fn builder_rejects_zero_capacity() {
    let hi = PushQueues::<u8>::builder().high_capacity(0).build();
    assert!(matches!(
        hi,
        Err(PushConfigError::InvalidCapacity { high: 0, low: 1 })
    ));

    let lo = PushQueues::<u8>::builder().low_capacity(0).build();
    assert!(matches!(
        lo,
        Err(PushConfigError::InvalidCapacity { high: 1, low: 0 })
    ));
}

/// Frames are delivered to queues matching their push priority.
#[tokio::test]
async fn frames_routed_to_correct_priority_queues() {
    let (mut queues, handle) = small_queues();

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
/// return [`PushError::QueueFull`] once the queue is at capacity.
#[tokio::test]
async fn try_push_respects_policy() {
    let (mut queues, handle) = small_queues();

    push_expect!(handle.push_high_priority(1u8));
    let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    assert!(matches!(result, Err(PushError::QueueFull)));

    // drain queue to allow new push
    let _ = queues.recv().await;
    push_expect!(handle.push_high_priority(3u8));
    let (_, last) = recv_expect!(queues.recv());
    assert_eq!(last, 3);
}

/// Push attempts return `Closed` when all queues have been shut down.
#[tokio::test]
async fn push_queues_error_on_closed() {
    let (mut queues, handle) = small_queues();
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
    let (mut queues, handle) = queues();

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
    time::pause();
    let (mut queues, handle) = queues();
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
    let (mut queues, handle) = queues();
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
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .rate(None)
        .build()
        .expect("failed to build PushQueues");
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
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(3))
        .build()
        .expect("failed to build PushQueues");

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
