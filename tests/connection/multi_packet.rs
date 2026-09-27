//! Multi-packet queue tests: forwarding, closed handling, and drain behaviour.

use rstest::rstest;
use tokio::sync::mpsc;
use wireframe_testing::TestResult;

use crate::support::{
    HarnessConfig,
    HarnessFactory,
    HookCounts,
    assert_frame_processed,
    assert_multi_packet_processing_result,
    harness_factory,
};

#[rstest]
fn process_multi_packet_forwards_frame(harness_factory: HarnessFactory) -> TestResult {
    let mut harness = harness_factory.create(HarnessConfig::new())?;
    harness.process_multi_packet(Some(5));

    assert_multi_packet_processing_result(
        &harness,
        &harness_factory,
        &[6],
        HookCounts { before: 1, end: 0 },
    );
    Ok(())
}

#[rstest]
fn process_multi_packet_none_emits_end_frame(harness_factory: HarnessFactory) -> TestResult {
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_increment(2)
            .with_stream_end(|_| Some(9)),
    )?;
    let (_tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx))?;

    harness.process_multi_packet(None);

    assert_multi_packet_processing_result(
        &harness,
        &harness_factory,
        &[11],
        HookCounts { before: 1, end: 1 },
    );
    Ok(())
}

#[rstest(
    terminator,
    expected_output,
    expected_before,
    case::with_terminator(Some(5), vec![6], 1),
    case::without_terminator(None, Vec::new(), 0),
)]
fn handle_multi_packet_closed_behaviour(
    harness_factory: HarnessFactory,
    terminator: Option<u8>,
    expected_output: Vec<u8>,
    expected_before: usize,
) -> TestResult {
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(move |_| terminator),
    )?;
    let (_tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx))?;

    harness.handle_multi_packet_closed();

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
    if harness.has_multi_queue() {
        return Err("multi-packet channel should be cleared".into());
    }
    assert_frame_processed(
        &harness.out,
        &expected_output,
        HookCounts {
            before: expected_before,
            end: 1,
        },
        harness_factory.counts(),
    );
    Ok(())
}

#[rstest]
fn try_opportunistic_drain_forwards_frame(harness_factory: HarnessFactory) -> TestResult {
    let mut harness = harness_factory.create(HarnessConfig::new())?;
    let (tx, rx) = mpsc::channel(1);
    tx.try_send(9)?;
    drop(tx);
    harness.set_low_queue(Some(rx));

    let drained = harness.try_drain_low();

    if !drained {
        return Err("queue should report a drained frame".into());
    }
    if !harness.has_low_queue() {
        return Err("queue remains available".into());
    }
    assert_frame_processed(
        &harness.out,
        &[10],
        HookCounts { before: 1, end: 0 },
        harness_factory.counts(),
    );
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
    let mut harness = wireframe::connection::test_support::ActorHarness::new()?;
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
    let mut harness = wireframe::connection::test_support::ActorHarness::new()?;
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
