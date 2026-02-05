#![cfg_attr(loom, allow(missing_docs))]
#![cfg(not(loom))]
//! Regression test for rate limiter token reservation behaviour.
//!
//! Ensures that probing a pending push future with `now_or_never()` does not
//! reserve limiter tokens in a way that starves subsequently polled tasks.
//! The third push must succeed immediately after the refill window even if the
//! second future is never polled again.

use futures::FutureExt;
use tokio::time::{self, Duration};
use wireframe::push::PushQueues;

#[tokio::test]
async fn limiter_does_not_reserve_tokens_for_unpolled_future() {
    time::pause();
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .rate(Some(1))
        .build()
        .expect("failed to build PushQueues");

    // First push consumes the single token in the current window.
    handle
        .push_high_priority(1u8)
        .await
        .expect("first push failed");

    // Create a second push future and probe it to register with the scheduler
    // without actually awaiting. This used to reserve the next token and
    // starve other pushes if the future was never polled again.
    let mut pending = handle.push_high_priority(2u8).boxed();
    tokio::task::yield_now().await;
    assert!(
        pending.as_mut().now_or_never().is_none(),
        "second push should be pending"
    );

    // Advance virtual time to the next window.
    time::advance(Duration::from_secs(1)).await;

    // The third push must succeed promptly; wrap in a timeout to avoid hangs.
    tokio::time::timeout(Duration::from_millis(50), handle.push_high_priority(3u8))
        .await
        .expect("third push timed out")
        .expect("third push failed");

    // Frames arrive in the expected order: first and third.
    let (_, a) = queues.recv().await.expect("recv failed");
    let (_, b) = queues.recv().await.expect("recv failed");
    assert_eq!((a, b), (1, 3));
}
