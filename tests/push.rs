#![cfg(not(loom))]
//! Tests for push queue routing and limits.
//!
//! They cover priority ordering, policy behaviour, and closed queue errors.

mod support;

use futures::FutureExt;
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
use wireframe_testing::{TestResult, push_expect, recv_expect};

#[fixture]
fn queues() -> Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError> {
    support::builder::<u8>()
        .high_capacity(2)
        .low_capacity(2)
        .rate(Some(1))
        .build()
}

#[fixture]
fn small_queues() -> Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError> {
    support::builder::<u8>().build()
}

/// Builder rejects rates outside the supported range.
#[rstest]
#[case::zero(0)]
#[case::too_high(MAX_PUSH_RATE + 1)]
fn builder_rejects_invalid_rate(#[case] rate: usize) {
    let result = support::builder::<u8>().rate(Some(rate)).build();
    assert!(matches!(result, Err(PushConfigError::InvalidRate(r)) if r == rate));
}

/// Setting the rate to `None` disables throttling without error.
#[test]
fn builder_accepts_none_rate() {
    let result = support::builder::<u8>().rate(None).build();
    assert!(result.is_ok(), "builder should accept None rate");
}

/// Builder accepts the maximum supported rate.
#[test]
fn builder_accepts_max_rate() {
    let result = support::builder::<u8>().rate(Some(MAX_PUSH_RATE)).build();
    assert!(result.is_ok(), "builder should accept MAX_PUSH_RATE");
}

/// Disabling throttling allows rapid bursts to succeed.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn disables_throttling_allows_burst_pushes() -> TestResult<()> {
    time::pause();
    let (_queues, handle) = support::builder::<u8>()
        .high_capacity(20)
        .low_capacity(20)
        .unlimited()
        .build()?;
    for i in 0u8..10 {
        push_expect!(handle.push_high_priority(i));
        push_expect!(handle.push_low_priority(i));
    }
    let res = time::timeout(Duration::from_millis(10), handle.push_high_priority(99)).await;
    let push_res = res.expect("push should not block when throttling disabled");
    assert!(
        push_res.is_ok(),
        "push should not error when throttling disabled"
    );
    Ok(())
}

#[test]
fn builder_rejects_zero_capacity() {
    let hi = support::builder::<u8>().high_capacity(0).build();
    assert!(matches!(
        hi,
        Err(PushConfigError::InvalidCapacity { high: 0, low: 1 })
    ));

    let lo = support::builder::<u8>().low_capacity(0).build();
    assert!(matches!(
        lo,
        Err(PushConfigError::InvalidCapacity { high: 1, low: 0 })
    ));

    let both = support::builder::<u8>()
        .high_capacity(0)
        .low_capacity(0)
        .build();
    assert!(matches!(
        both,
        Err(PushConfigError::InvalidCapacity { high: 0, low: 0 })
    ));
}

/// Frames are delivered to queues matching their push priority.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn frames_routed_to_correct_priority_queues() -> TestResult<()> {
    let (mut queues, handle) = small_queues()?;

    push_expect!(handle.push_low_priority(1u8));
    push_expect!(handle.push_high_priority(2u8));

    let (prio1, frame1) = recv_expect!(queues.recv());
    let (prio2, frame2) = recv_expect!(queues.recv());

    assert_eq!(
        prio1,
        PushPriority::High,
        "first frame should be high priority"
    );
    assert_eq!(frame1, 2, "unexpected first frame value");
    assert_eq!(
        prio2,
        PushPriority::Low,
        "second frame should be low priority"
    );
    assert_eq!(frame2, 1, "unexpected second frame value");
    Ok(())
}

/// `try_push` honours the selected queue policy when full.
///
/// Using [`PushPolicy::ReturnErrorIfFull`] causes `try_push` to
/// return [`PushError::QueueFull`] once the queue is at capacity.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn try_push_respects_policy() -> TestResult<()> {
    let (mut queues, handle) = small_queues()?;

    push_expect!(handle.push_high_priority(1u8));
    let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    assert!(
        matches!(result, Err(PushError::QueueFull)),
        "expected queue full error"
    );

    // drain queue to allow new push
    let _ = queues.recv().await;
    push_expect!(handle.push_high_priority(3u8));
    let (_, last) = recv_expect!(queues.recv());
    assert_eq!(last, 3, "unexpected drained frame");
    Ok(())
}

/// Push attempts return `Closed` when all queues have been shut down.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn push_queues_error_on_closed() -> TestResult<()> {
    let (mut queues, handle) = small_queues()?;
    queues.close();
    let res = handle.push_high_priority(42u8).await;
    assert!(
        matches!(res, Err(PushError::Closed)),
        "expected closed error on high priority push"
    );

    let res = handle.push_low_priority(24u8).await;
    assert!(
        matches!(res, Err(PushError::Closed)),
        "expected closed error on low priority push"
    );
    Ok(())
}

/// A push beyond the configured rate is blocked.
/// Time is paused using [`tokio::time::pause`], so the test runs in a
/// virtual-time context.
#[rstest]
#[case::high(PushPriority::High)]
#[case::low(PushPriority::Low)]
#[tokio::test]
async fn rate_limiter_blocks_when_exceeded(#[case] priority: PushPriority) -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = queues()?;

    match priority {
        PushPriority::High => push_expect!(handle.push_high_priority(1u8)),
        PushPriority::Low => push_expect!(handle.push_low_priority(1u8)),
    }

    let mut fut = match priority {
        PushPriority::High => handle.push_high_priority(2u8).boxed(),
        PushPriority::Low => handle.push_low_priority(2u8).boxed(),
    };
    tokio::task::yield_now().await; // register w/ scheduler
    assert!(
        fut.as_mut().now_or_never().is_none(),
        "second push should be pending under rate limit"
    );

    time::advance(Duration::from_secs(1)).await;
    match priority {
        PushPriority::High => push_expect!(handle.push_high_priority(3u8)),
        PushPriority::Low => push_expect!(handle.push_low_priority(3u8)),
    }

    let (_, first) = recv_expect!(queues.recv());
    let (_, second) = recv_expect!(queues.recv());
    assert_eq!(
        (first, second),
        (1, 3),
        "unexpected drained frames under rate limit"
    );
    Ok(())
}

/// Exceeding the rate limit succeeds after the window has passed.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn rate_limiter_allows_after_wait() -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = queues()?;
    push_expect!(handle.push_high_priority(1u8));
    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_high_priority(2u8));

    let (_, a) = recv_expect!(queues.recv());
    let (_, b) = recv_expect!(queues.recv());
    assert_eq!((a, b), (1, 2), "unexpected frame ordering after wait");
    Ok(())
}

/// The limiter counts pushes from all priority queues.
/// The token bucket is shared, so pushes from one priority reduce
/// the allowance for the other.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn rate_limiter_shared_across_priorities() -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = queues()?;
    push_expect!(handle.push_high_priority(1u8));

    let mut fut = handle.push_low_priority(2u8).boxed();
    tokio::task::yield_now().await;
    assert!(
        fut.as_mut().now_or_never().is_none(),
        "second push should be pending across queues"
    );

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_low_priority(2u8));

    let (prio1, frame1) = recv_expect!(queues.recv());
    let (prio2, frame2) = recv_expect!(queues.recv());
    assert_eq!(prio1, PushPriority::High, "first priority should be high");
    assert_eq!(frame1, 1, "unexpected first frame value");
    assert_eq!(prio2, PushPriority::Low, "second priority should be low");
    assert_eq!(frame2, 2, "unexpected second frame value");
    Ok(())
}

/// Unlimited queues never block pushes.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn unlimited_queues_do_not_block() -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = support::builder::<u8>().unlimited().build()?;
    push_expect!(handle.push_high_priority(1u8));
    let res = time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await;
    assert!(res.is_ok(), "pushes should not block when unlimited");

    let (_, a) = recv_expect!(queues.recv());
    let (_, b) = recv_expect!(queues.recv());
    assert_eq!((a, b), (1, 2), "unexpected ordering for unlimited queues");
    Ok(())
}

/// A burst up to capacity succeeds and further pushes are blocked.
/// The maximum burst size equals the configured `capacity` parameter.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn rate_limiter_allows_burst_within_capacity_and_blocks_excess() -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = support::builder::<u8>()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(3))
        .build()?;

    for i in 0u8..3 {
        push_expect!(handle.push_high_priority(i));
    }

    let mut fut = handle.push_high_priority(99).boxed();
    tokio::task::yield_now().await;
    assert!(
        fut.as_mut().now_or_never().is_none(),
        "push exceeding burst capacity should be pending"
    );

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_high_priority(100));

    for expected in [0u8, 1u8, 2u8, 100u8] {
        let (_, frame) = recv_expect!(queues.recv());
        assert_eq!(
            frame, expected,
            "frames drained in unexpected order: expected {expected}, got {frame}"
        );
    }
    Ok(())
}
