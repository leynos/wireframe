//! Tests for connection shutdown, queue polling, and diagnostic reasons.

use super::*;

#[rstest]
#[serial(connection_logs)]
fn handle_multi_packet_closed_logs_reason(
    harness_factory: HarnessFactory,
    mut logger: LoggerHandle,
) -> TestResult {
    logger.clear();
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    )?;
    let (_tx, rx) = mpsc::channel(1);
    harness
        .actor_mut()
        .set_multi_packet_with_correlation(Some(rx), Some(11))?;
    logger.clear();
    harness.handle_multi_packet_closed();
    assert_reason_logged(&mut logger, Level::Info, "drained", Some(11));
    Ok(())
}

#[rstest]
#[serial(connection_logs)]
fn try_opportunistic_drain_multi_disconnect_logs_reason(
    harness_factory: HarnessFactory,
    mut logger: LoggerHandle,
) -> TestResult {
    logger.clear();
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    )?;
    let (tx, rx) = mpsc::channel(1);
    harness
        .actor_mut()
        .set_multi_packet_with_correlation(Some(rx), Some(12))?;
    drop(tx);
    logger.clear();
    let drained = harness.try_drain_multi();
    if drained {
        return Err("disconnect should not report a drained frame".into());
    }
    assert_reason_logged(&mut logger, Level::Warn, "disconnected", Some(12));
    Ok(())
}

#[rstest]
#[serial(connection_logs)]
fn start_shutdown_logs_reason(
    harness_factory: HarnessFactory,
    mut logger: LoggerHandle,
) -> TestResult {
    logger.clear();
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    )?;
    let (_tx, rx) = mpsc::channel(1);
    harness
        .actor_mut()
        .set_multi_packet_with_correlation(Some(rx), Some(13))?;
    logger.clear();
    harness.start_shutdown();
    assert_reason_logged(&mut logger, Level::Info, "shutdown", Some(13));
    if harness.has_multi_queue() {
        return Err("multi-packet queue should be cleared after shutdown".into());
    }
    Ok(())
}

#[rstest]
fn try_opportunistic_drain_multi_disconnect_emits_terminator(
    harness_factory: HarnessFactory,
) -> TestResult {
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    )?;
    let (tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx))?;
    drop(tx);

    let drained = harness.try_drain_multi();

    if drained {
        return Err("disconnect should not report a drained frame".into());
    }
    if harness.has_multi_queue() {
        return Err("multi-packet queue should be cleared after disconnect".into());
    }
    assert_frame_processed(
        &harness.out,
        &[6],
        HookCounts { before: 1, end: 1 },
        harness_factory.counts(),
    );
    Ok(())
}

#[test]
fn try_opportunistic_drain_returns_false_when_empty() -> TestResult {
    let mut harness = ActorHarness::new()?;
    let (_tx, rx) = mpsc::channel(1);
    harness.set_low_queue(Some(rx));

    let drained = harness.try_drain_low();

    if drained {
        return Err("no frame should be drained".into());
    }
    if !harness.has_low_queue() {
        return Err("queue should remain available".into());
    }
    if !harness.out.is_empty() {
        return Err("no frames should be emitted".into());
    }
    Ok(())
}

#[test]
fn try_opportunistic_drain_handles_disconnect() -> TestResult {
    let mut harness = ActorHarness::new()?;
    let (tx, rx) = mpsc::channel(1);
    harness.set_low_queue(Some(rx));
    drop(tx);

    let drained = harness.try_drain_low();

    if drained {
        return Err("disconnect should not produce a frame".into());
    }
    if harness.has_low_queue() {
        return Err("queue should be cleared after disconnect".into());
    }
    let snapshot = harness.snapshot();
    if !snapshot.is_active {
        return Err("connection should be active".into());
    }
    if snapshot.is_shutting_down {
        return Err("connection should not be shutting down".into());
    }
    if snapshot.is_done {
        return Err("connection should not be done".into());
    }
    Ok(())
}

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
