//! Queue polling and actor lifecycle-state tests.

use rstest::rstest;
use tokio::sync::mpsc;
use wireframe::connection::test_support::{ActorStateHarness, poll_queue_next};

#[tokio::test]
async fn poll_queue_reads_frame() {
    let (tx, mut rx) = mpsc::channel(1);
    tx.send(42).await.expect("send frame");

    let value = poll_queue_next(Some(&mut rx)).await;

    assert_eq!(value, Some(42));
}

#[tokio::test]
async fn poll_queue_returns_none_for_absent_receiver() {
    let value = poll_queue_next(None).await;
    assert!(value.is_none());
}

#[tokio::test]
async fn poll_queue_returns_none_after_close() {
    let (tx, mut rx) = mpsc::channel(1);
    drop(tx);

    let value = poll_queue_next(Some(&mut rx)).await;

    assert!(value.is_none());
}

#[test]
fn actor_state_reports_shutting_down_after_start_shutdown() {
    // `is_shutting_down` is only ever asserted false elsewhere; drive the
    // transition and assert the positive polarity so a constant-`false`
    // mutant cannot survive.
    let mut harness = ActorStateHarness::new(false, false);
    assert!(!harness.snapshot().is_shutting_down, "should start active");

    harness.start_shutdown();
    let snapshot = harness.snapshot();
    assert!(snapshot.is_shutting_down, "should be shutting down");
    assert!(!snapshot.is_active, "shutting down is not active");
    assert!(!snapshot.is_done, "shutting down is not done");
}

#[rstest(
    has_multi,
    expected_marks,
    case::with_multi(true, 3),
    case::without_multi(false, 2)
)]
fn actor_state_tracks_sources(has_multi: bool, expected_marks: usize) {
    let mut harness = ActorStateHarness::new(false, has_multi);
    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);

    for _ in 0..expected_marks.saturating_sub(1) {
        harness.mark_closed();
        let snapshot = harness.snapshot();
        assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
    }

    harness.mark_closed();
    let final_snapshot = harness.snapshot();
    assert!(
        !final_snapshot.is_active && !final_snapshot.is_shutting_down && final_snapshot.is_done
    );
}
