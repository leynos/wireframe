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
use wireframe_testing::{push_expect, recv_expect};

mod common;
use common::TestResult;

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
    if res.is_err() {
        return Err("push should not block when throttling disabled".into());
    }
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
async fn frames_routed_to_correct_priority_queues() -> TestResult<()> {
    let (mut queues, handle) = small_queues()?;

    push_expect!(handle.push_low_priority(1u8));
    push_expect!(handle.push_high_priority(2u8));

    let (prio1, frame1) = recv_expect!(queues.recv());
    let (prio2, frame2) = recv_expect!(queues.recv());

    if prio1 != PushPriority::High || frame1 != 2 {
        return Err("unexpected first frame ordering".into());
    }
    if prio2 != PushPriority::Low || frame2 != 1 {
        return Err("unexpected second frame ordering".into());
    }
    Ok(())
}

/// `try_push` honours the selected queue policy when full.
///
/// Using [`PushPolicy::ReturnErrorIfFull`] causes `try_push` to
/// return [`PushError::QueueFull`] once the queue is at capacity.
#[tokio::test]
async fn try_push_respects_policy() -> TestResult<()> {
    let (mut queues, handle) = small_queues()?;

    push_expect!(handle.push_high_priority(1u8));
    let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    if !matches!(result, Err(PushError::QueueFull)) {
        return Err("expected queue full error".into());
    }

    // drain queue to allow new push
    let _ = queues.recv().await;
    push_expect!(handle.push_high_priority(3u8));
    let (_, last) = recv_expect!(queues.recv());
    if last != 3 {
        return Err("unexpected drained frame".into());
    }
    Ok(())
}

/// Push attempts return `Closed` when all queues have been shut down.
#[tokio::test]
async fn push_queues_error_on_closed() -> TestResult<()> {
    let (mut queues, handle) = small_queues()?;
    queues.close();
    let res = handle.push_high_priority(42u8).await;
    if !matches!(res, Err(PushError::Closed)) {
        return Err("expected closed error on high priority push".into());
    }

    let res = handle.push_low_priority(24u8).await;
    if !matches!(res, Err(PushError::Closed)) {
        return Err("expected closed error on low priority push".into());
    }
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
    if fut.as_mut().now_or_never().is_some() {
        return Err("second push should be pending under rate limit".into());
    }

    time::advance(Duration::from_secs(1)).await;
    match priority {
        PushPriority::High => push_expect!(handle.push_high_priority(3u8)),
        PushPriority::Low => push_expect!(handle.push_low_priority(3u8)),
    }

    let (_, first) = recv_expect!(queues.recv());
    let (_, second) = recv_expect!(queues.recv());
    if (first, second) != (1, 3) {
        return Err(format!(
            "unexpected drained frames under rate limit: expected (1, 3), got ({first}, {second})"
        )
        .into());
    }
    Ok(())
}

/// Exceeding the rate limit succeeds after the window has passed.
#[tokio::test]
async fn rate_limiter_allows_after_wait() -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = queues()?;
    push_expect!(handle.push_high_priority(1u8));
    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_high_priority(2u8));

    let (_, a) = recv_expect!(queues.recv());
    let (_, b) = recv_expect!(queues.recv());
    if (a, b) != (1, 2) {
        return Err(format!("unexpected frame ordering after wait: ({a}, {b})").into());
    }
    Ok(())
}

/// The limiter counts pushes from all priority queues.
/// The token bucket is shared, so pushes from one priority reduce
/// the allowance for the other.
#[tokio::test]
async fn rate_limiter_shared_across_priorities() -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = queues()?;
    push_expect!(handle.push_high_priority(1u8));

    let mut fut = handle.push_low_priority(2u8).boxed();
    tokio::task::yield_now().await;
    if fut.as_mut().now_or_never().is_some() {
        return Err("second push should be pending across queues".into());
    }

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_low_priority(2u8));

    let (prio1, frame1) = recv_expect!(queues.recv());
    let (prio2, frame2) = recv_expect!(queues.recv());
    if prio1 != PushPriority::High || prio2 != PushPriority::Low {
        return Err(format!(
            "unexpected priorities: first={prio1:?}, second={prio2:?} (expected High then Low)"
        )
        .into());
    }
    if (frame1, frame2) != (1, 2) {
        return Err(format!("unexpected frame values: {frame1}, {frame2}").into());
    }
    Ok(())
}

/// Unlimited queues never block pushes.
#[tokio::test]
async fn unlimited_queues_do_not_block() -> TestResult<()> {
    time::pause();
    let (mut queues, handle) = support::builder::<u8>().unlimited().build()?;
    push_expect!(handle.push_high_priority(1u8));
    let res = time::timeout(Duration::from_millis(10), handle.push_low_priority(2u8)).await;
    if res.is_err() {
        return Err("pushes should not block when unlimited".into());
    }

    let (_, a) = recv_expect!(queues.recv());
    let (_, b) = recv_expect!(queues.recv());
    if (a, b) != (1, 2) {
        return Err(format!("unexpected ordering for unlimited queues: ({a}, {b})").into());
    }
    Ok(())
}

/// A burst up to capacity succeeds and further pushes are blocked.
/// The maximum burst size equals the configured `capacity` parameter.
#[tokio::test]
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
    if fut.as_mut().now_or_never().is_some() {
        return Err("push exceeding burst capacity should be pending".into());
    }

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_high_priority(100));

    for expected in [0u8, 1u8, 2u8, 100u8] {
        let (_, frame) = recv_expect!(queues.recv());
        if frame != expected {
            return Err(format!(
                "frames drained in unexpected order: expected {expected}, got {frame}"
            )
            .into());
        }
    }
    Ok(())
}
