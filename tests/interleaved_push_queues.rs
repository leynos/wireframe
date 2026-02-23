//! Tests proving interleaved high- and low-priority push queue fairness
//! and rate-limit symmetry.
//!
//! These tests exercise the `ConnectionActor`'s biased `select!` loop under
//! various `FairnessConfig` settings and shared rate-limiter configurations
//! to prove that:
//!
//! 1. The rate limiter enforces identical caps regardless of priority.
//! 2. Fairness thresholds cause low-priority frames to be interleaved.
//! 3. No frames are lost under concurrent interleaved traffic.
#![cfg(not(loom))]

use std::{future::Future, pin::Pin};

use futures::FutureExt;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::{
    sync::oneshot,
    time::{self, Duration},
};
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::{ConnectionActor, FairnessConfig},
    push::{PushError, PushHandle, PushQueues},
};
use wireframe_testing::{TestResult, push_expect};

#[path = "common/interleaved_push_helpers.rs"]
mod interleaved_push_helpers;

// rustfmt collapses simple fixtures into one line, which triggers
// unused_braces.
#[rustfmt::skip]
#[fixture]
fn shutdown_token() -> CancellationToken {
    CancellationToken::new()
}

/// Build unlimited queues, load frames via `setup`, run the actor with
/// the given `fairness` config, and pass collected output to `assertions`.
async fn run_actor_test<S, SFut, A>(fairness: FairnessConfig, setup: S, assertions: A) -> TestResult
where
    S: FnOnce(PushHandle<u8>) -> SFut,
    SFut: Future<Output = ()>,
    A: FnOnce(Vec<u8>),
{
    let out = interleaved_push_helpers::run_actor_with_fairness(fairness, setup).await?;
    assertions(out);
    Ok(())
}

// ── Rate-limit symmetry ─────────────────────────────────────────────

/// Type alias for a push function that takes a handle reference and a
/// frame value, returning a boxed future resolving to `PushError`.
type PushFn = fn(&PushHandle<u8>, u8) -> Pin<Box<dyn Future<Output = Result<(), PushError>> + '_>>;

/// Build rate-limited queues (rate=2), push a burst that saturates the
/// token bucket, verify the next push blocks, advance time, push once
/// more, and drain to confirm delivery order.
async fn test_single_priority_rate_limit(push: PushFn, priority_name: &str) -> TestResult {
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(2))
        .build()?;

    // Burst of 2 should succeed immediately.
    push(&handle, 1).await?;
    push(&handle, 2).await?;

    // Third push should be blocked by the rate limiter.
    let mut blocked = push(&handle, 3);
    tokio::task::yield_now().await;
    assert!(
        blocked.as_mut().now_or_never().is_none(),
        "third {priority_name} push should be pending under rate limit"
    );

    // Advance past the refill window and push again.
    time::advance(Duration::from_secs(1)).await;
    push(&handle, 4).await?;

    // Drain to confirm frame delivery.
    let frames = drain_three(&mut queues).await?;
    assert_eq!(
        frames,
        (1, 2, 4),
        "{priority_name} frames delivered in order"
    );
    Ok(())
}

/// Drain exactly three frames from the queue, returning them as a tuple.
async fn drain_three(queues: &mut PushQueues<u8>) -> TestResult<(u8, u8, u8)> {
    let (_, a) = queues.recv().await.ok_or("recv 1 failed")?;
    let (_, b) = queues.recv().await.ok_or("recv 2 failed")?;
    let (_, c) = queues.recv().await.ok_or("recv 3 failed")?;
    Ok((a, b, c))
}

/// The rate limiter blocks high-priority pushes after the burst window
/// is exhausted, proving it applies to the high queue.
#[rstest]
#[tokio::test]
#[serial]
async fn rate_limit_symmetric_high_only() -> TestResult {
    test_single_priority_rate_limit(|h, v| Box::pin(h.push_high_priority(v)), "high").await
}

/// The rate limiter blocks low-priority pushes after the burst window
/// is exhausted, proving it applies to the low queue identically.
#[rstest]
#[tokio::test]
#[serial]
async fn rate_limit_symmetric_low_only() -> TestResult {
    test_single_priority_rate_limit(|h, v| Box::pin(h.push_low_priority(v)), "low").await
}

/// A high-priority push consumes a token from the shared bucket,
/// blocking a subsequent low-priority push until the next refill.
#[rstest]
#[tokio::test]
#[serial]
async fn rate_limit_symmetric_mixed() -> TestResult {
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(1))
        .build()?;

    // High push consumes the single token.
    push_expect!(handle.push_high_priority(1));

    // Low push should now be blocked.
    let mut blocked = handle.push_low_priority(2).boxed();
    tokio::task::yield_now().await;
    assert!(
        blocked.as_mut().now_or_never().is_none(),
        "low push should be pending: high already consumed the token"
    );

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_low_priority(3));

    let (_, a) = queues.recv().await.ok_or("recv 1 failed")?;
    let (_, b) = queues.recv().await.ok_or("recv 2 failed")?;
    assert_eq!(a, 1, "first frame should be the high push");
    assert_eq!(b, 3, "second frame should be the low push after refill");
    Ok(())
}

// ── Fairness interleaving ───────────────────────────────────────────

/// With `max_high_before_low = 3`, low-priority frames are interleaved
/// every 3 high-priority frames.
#[rstest]
#[tokio::test]
#[serial]
async fn interleaved_fairness_yields_at_threshold() -> TestResult {
    run_actor_test(
        FairnessConfig {
            max_high_before_low: 3,
            time_slice: None,
        },
        // Pre-load: 6 high, 2 low.
        |handle| async move {
            for n in 1..=6 {
                push_expect!(handle.push_high_priority(n));
            }
            push_expect!(handle.push_low_priority(101));
            push_expect!(handle.push_low_priority(102));
        },
        // Expected: H H H L H H H L
        |out| {
            assert_eq!(
                out,
                vec![1, 2, 3, 101, 4, 5, 6, 102],
                "low-priority frames should be interleaved every 3 high frames"
            );
        },
    )
    .await
}

/// All frames are delivered when both queues carry traffic with
/// fairness enabled. No frame loss occurs.
#[rstest]
#[tokio::test]
#[serial]
async fn interleaved_all_frames_delivered() -> TestResult {
    run_actor_test(
        FairnessConfig {
            max_high_before_low: 2,
            time_slice: None,
        },
        |handle| async move {
            for n in 1..=5 {
                push_expect!(handle.push_high_priority(n));
            }
            for n in 101..=105 {
                push_expect!(handle.push_low_priority(n));
            }
        },
        |out| {
            assert_eq!(out.len(), 10, "all 10 frames should be delivered");

            let mut high_frames: Vec<u8> = out.iter().copied().filter(|&f| f <= 5).collect();
            let mut low_frames: Vec<u8> = out.iter().copied().filter(|&f| f >= 101).collect();
            high_frames.sort_unstable();
            low_frames.sort_unstable();
            assert_eq!(high_frames, vec![1, 2, 3, 4, 5], "all high frames present");
            assert_eq!(
                low_frames,
                vec![101, 102, 103, 104, 105],
                "all low frames present"
            );
        },
    )
    .await
}

/// Time-slice fairness yields to low-priority traffic after the
/// configured duration, even with the counter threshold disabled.
#[rstest]
#[tokio::test]
#[serial]
async fn interleaved_time_slice_fairness(shutdown_token: CancellationToken) -> TestResult {
    time::pause();
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .unlimited()
        .build()?;

    let fairness = FairnessConfig {
        max_high_before_low: 0,
        time_slice: Some(Duration::from_millis(10)),
    };

    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle.clone(), None, shutdown_token);
    actor.set_fairness(fairness);

    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut out = Vec::new();
        let _ = actor.run(&mut out).await;
        let _ = tx.send(out);
    });

    push_expect!(handle.push_high_priority(1));
    time::advance(Duration::from_millis(5)).await;
    push_expect!(handle.push_high_priority(2));
    // Advance past the time slice so the next high frame triggers yield.
    time::advance(Duration::from_millis(15)).await;
    push_expect!(handle.push_low_priority(42));
    for n in 3..=5 {
        push_expect!(handle.push_high_priority(n));
    }
    drop(handle);

    let out = rx.await.map_err(|_| "actor output missing")?;
    assert!(out.contains(&42), "low-priority item should be delivered");
    let pos = out
        .iter()
        .position(|x| *x == 42)
        .ok_or("value 42 should be present")?;
    assert!(
        pos > 0 && pos < out.len() - 1,
        "low-priority item should be yielded mid-stream: pos={pos}, out={out:?}"
    );
    Ok(())
}

/// With rate R=4, the total throughput across both queues is capped at
/// 4 per second, proving the token bucket is shared.
#[rstest]
#[tokio::test]
#[serial]
async fn rate_limit_interleaved_total_throughput() -> TestResult {
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .rate(Some(4))
        .build()?;

    // First 4 pushes (mix of priorities) should succeed immediately.
    push_expect!(handle.push_high_priority(1));
    push_expect!(handle.push_low_priority(2));
    push_expect!(handle.push_high_priority(3));
    push_expect!(handle.push_low_priority(4));

    // The 5th push (either priority) should block.
    let mut blocked = handle.push_high_priority(5).boxed();
    tokio::task::yield_now().await;
    assert!(
        blocked.as_mut().now_or_never().is_none(),
        "5th push should be pending: all 4 tokens consumed"
    );

    time::advance(Duration::from_secs(1)).await;
    push_expect!(handle.push_high_priority(6));
    push_expect!(handle.push_low_priority(7));

    // Drain all frames to verify delivery.
    let mut out = Vec::new();
    for _ in 0..6 {
        let (_, frame) = queues.recv().await.ok_or("recv failed")?;
        out.push(frame);
    }
    assert_eq!(out.len(), 6, "all 6 accepted frames should be delivered");
    Ok(())
}

/// With fairness disabled (counter=0, no time slice), the biased
/// select! loop processes all high-priority frames before any
/// low-priority frames.
#[rstest]
#[tokio::test]
#[serial]
async fn fairness_disabled_strict_priority() -> TestResult {
    run_actor_test(
        FairnessConfig {
            max_high_before_low: 0,
            time_slice: None,
        },
        |handle| async move {
            push_expect!(handle.push_low_priority(101));
            push_expect!(handle.push_low_priority(102));
            push_expect!(handle.push_high_priority(1));
            push_expect!(handle.push_high_priority(2));
            push_expect!(handle.push_high_priority(3));
        },
        |out| {
            assert_eq!(
                out,
                vec![1, 2, 3, 101, 102],
                "all high frames should precede all low frames"
            );
        },
    )
    .await
}
