//! Tests for push queue routing and limits.
//!
//! They cover priority ordering, policy behaviour, and closed queue errors.

use rstest::rstest;
use tokio::time::{self, Duration};
use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};

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

/// Tests that the rate limiter blocks pushes exceeding the configured rate limit for both
/// high and low priorities.
///
/// After pushing one frame, a second push attempt is made and expected to time out,
/// demonstrating that the rate limiter enforces blocking. Advancing time allows a
/// subsequent push, and the test verifies that only the allowed frames are received.
///
/// # Examples
///
/// ```no_run
/// // This test is parameterised and runs for both high and low priorities.
/// // It verifies that the rate limiter blocks excess pushes and allows them after waiting.
/// ```
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

/// Tests that the rate limiter allows pushes after the wait interval has elapsed.
///
/// This test verifies that after pushing a frame and waiting for the rate limiter's interval,
/// a subsequent push is permitted. It ensures that both frames are received in the correct order.
///
/// # Examples
///
/// ```no_run
/// # use wireframe::push::PushQueues;
/// # use tokio::time::{self, Duration};
/// # #[tokio::main]
/// # async fn main() {
/// time::pause();
/// let (mut queues, handle) = PushQueues::bounded_with_rate(2, 2, Some(1)).unwrap();
/// handle.push_high_priority(1u8).await.unwrap();
/// time::advance(Duration::from_secs(1)).await;
/// handle.push_high_priority(2u8).await.unwrap();
///
/// let (_, a) = queues.recv().await.unwrap();
/// let (_, b) = queues.recv().await.unwrap();
/// assert_eq!((a, b), (1, 2));
/// # }
/// ```
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

/// Tests that the rate limiter is enforced across both high and low priority queues.
///
/// Verifies that after pushing a high priority frame, a low priority push attempt blocks
/// until the rate limiter interval elapses, after which the push succeeds. Ensures that
/// the rate limiter is shared between priorities and that frames are received in the
/// correct order.
///
/// # Examples
///
/// ```no_run
/// # use wireframe::push::{PushQueues, PushPriority};
/// # use tokio::time::{self, Duration};
/// # #[tokio::main]
/// # async fn main() {
/// time::pause();
/// let (mut queues, handle) = PushQueues::bounded_with_rate(2, 2, Some(1)).unwrap();
/// handle.push_high_priority(1u8).await.unwrap();
///
/// let attempt = time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await;
/// assert!(attempt.is_err());
///
/// time::advance(Duration::from_secs(1)).await;
/// handle.push_low_priority(2u8).await.unwrap();
///
/// let (prio1, frame1) = queues.recv().await.unwrap();
/// let (prio2, frame2) = queues.recv().await.unwrap();
/// assert_eq!(prio1, PushPriority::High);
/// assert_eq!(frame1, 1);
/// assert_eq!(prio2, PushPriority::Low);
/// assert_eq!(frame2, 2);
/// # }
/// ```
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

/// Tests that the rate limiter allows a burst of pushes within its configured capacity and blocks any excess until the interval elapses.
///
/// This test pushes frames up to the burst capacity, verifies that an additional push is rate limited (blocked), then advances time to allow further pushes. It confirms that all frames are received in the expected order.
///
/// # Examples
///
/// ```no_run
/// # use wireframe::push::{PushQueues, PushPriority};
/// # use tokio::time::{self, Duration};
/// # #[tokio::main]
/// # async fn main() {
/// time::pause();
/// let (mut queues, handle) = PushQueues::bounded_with_rate(4, 4, Some(3)).unwrap();
///
/// for i in 0u8..3 {
///     handle.push_high_priority(i).await.unwrap();
/// }
///
/// let res = time::timeout(Duration::from_millis(10), handle.push_high_priority(99)).await;
/// assert!(res.is_err());
///
/// time::advance(Duration::from_secs(1)).await;
/// handle.push_high_priority(100).await.unwrap();
///
/// for expected in [0u8, 1u8, 2u8, 100u8] {
///     let (_, frame) = queues.recv().await.unwrap();
///     assert_eq!(frame, expected);
/// }
/// # }
/// ```
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
