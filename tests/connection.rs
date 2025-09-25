#![cfg(not(loom))]
//! Integration tests for connection actor queue helpers and multi-packet handling.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rstest::rstest;
use tokio::sync::mpsc;
use wireframe::{
    ProtocolHooks,
    connection::test_support::{ActorHarness, ActorStateHarness, poll_queue_next},
    hooks::ConnectionContext,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HookCounts {
    before: usize,
    end: usize,
}

impl HookCounts {
    /// Snapshot hook counters from shared state.
    fn from_counters(before: &Arc<AtomicUsize>, end: Option<&Arc<AtomicUsize>>) -> Self {
        let end = end.map_or(0, |counter| counter.load(Ordering::SeqCst));
        Self {
            before: before.load(Ordering::SeqCst),
            end,
        }
    }
}

fn assert_frame_processed(
    out: &[u8],
    expected: &[u8],
    expected_counts: HookCounts,
    actual_counts: HookCounts,
) {
    assert_eq!(out, expected, "frames should match expected output");
    assert_eq!(actual_counts, expected_counts, "hook counts should match");
}

#[test]
fn process_multi_packet_forwards_frame() {
    let before_calls = Arc::new(AtomicUsize::new(0));
    let before_clone = before_calls.clone();
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(
            move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                before_clone.fetch_add(1, Ordering::SeqCst);
                *frame += 1;
            },
        )),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut harness =
        ActorHarness::new_with_state(hooks, false, false).expect("failed to create harness");
    harness.process_multi_packet(Some(5));

    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
    assert_frame_processed(
        &harness.out,
        &[6],
        HookCounts { before: 1, end: 0 },
        HookCounts::from_counters(&before_calls, None),
    );
}

#[test]
fn process_multi_packet_none_emits_end_frame() {
    let before_calls = Arc::new(AtomicUsize::new(0));
    let before_clone = before_calls.clone();
    let end_calls = Arc::new(AtomicUsize::new(0));
    let end_clone = end_calls.clone();
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(
            move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                before_clone.fetch_add(1, Ordering::SeqCst);
                *frame += 2;
            },
        )),
        on_command_end: Some(Box::new(move |_ctx: &mut ConnectionContext| {
            end_clone.fetch_add(1, Ordering::SeqCst);
        })),
        stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| Some(9))),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut harness =
        ActorHarness::new_with_state(hooks, false, true).expect("failed to create harness");
    let (_tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx));

    harness.process_multi_packet(None);

    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
    assert_frame_processed(
        &harness.out,
        &[11],
        HookCounts { before: 1, end: 1 },
        HookCounts::from_counters(&before_calls, Some(&end_calls)),
    );
}

#[rstest(
    terminator,
    expected_output,
    expected_before,
    case::with_terminator(Some(5), vec![6], 1),
    case::without_terminator(None, Vec::new(), 0),
)]
fn handle_multi_packet_closed_behaviour(
    terminator: Option<u8>,
    expected_output: Vec<u8>,
    expected_before: usize,
) {
    let before_calls = Arc::new(AtomicUsize::new(0));
    let before_clone = before_calls.clone();
    let end_calls = Arc::new(AtomicUsize::new(0));
    let end_clone = end_calls.clone();
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(
            move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                before_clone.fetch_add(1, Ordering::SeqCst);
                *frame += 1;
            },
        )),
        on_command_end: Some(Box::new(move |_ctx: &mut ConnectionContext| {
            end_clone.fetch_add(1, Ordering::SeqCst);
        })),
        stream_end: Some(Box::new(move |_ctx: &mut ConnectionContext| terminator)),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut harness =
        ActorHarness::new_with_state(hooks, false, true).expect("failed to create harness");
    let (_tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx));

    harness.handle_multi_packet_closed();

    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
    assert!(
        !harness.has_multi_queue(),
        "multi-packet channel should be cleared"
    );
    assert_frame_processed(
        &harness.out,
        &expected_output,
        HookCounts {
            before: expected_before,
            end: 1,
        },
        HookCounts::from_counters(&before_calls, Some(&end_calls)),
    );
}

#[test]
fn try_opportunistic_drain_forwards_frame() {
    let before_calls = Arc::new(AtomicUsize::new(0));
    let before_clone = before_calls.clone();
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(
            move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                before_clone.fetch_add(1, Ordering::SeqCst);
                *frame += 1;
            },
        )),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut harness =
        ActorHarness::new_with_state(hooks, false, false).expect("failed to create harness");
    let (tx, rx) = mpsc::channel(1);
    tx.try_send(9).expect("send frame");
    drop(tx);
    harness.set_low_queue(Some(rx));

    let drained = harness.try_drain_low();

    assert!(drained, "queue should report a drained frame");
    assert!(harness.has_low_queue(), "queue remains available");
    assert_frame_processed(
        &harness.out,
        &[10],
        HookCounts { before: 1, end: 0 },
        HookCounts::from_counters(&before_calls, None),
    );
}

#[test]
fn try_opportunistic_drain_returns_false_when_empty() {
    let mut harness = ActorHarness::new().expect("failed to create harness");
    let (_tx, rx) = mpsc::channel(1);
    harness.set_low_queue(Some(rx));

    let drained = harness.try_drain_low();

    assert!(!drained, "no frame should be drained");
    assert!(harness.has_low_queue(), "queue should remain available");
    assert!(harness.out.is_empty(), "no frames should be emitted");
}

#[test]
fn try_opportunistic_drain_handles_disconnect() {
    let mut harness = ActorHarness::new().expect("failed to create harness");
    let (tx, rx) = mpsc::channel(1);
    harness.set_low_queue(Some(rx));
    drop(tx);

    let drained = harness.try_drain_low();

    assert!(!drained, "disconnect should not produce a frame");
    assert!(
        !harness.has_low_queue(),
        "queue should be cleared after disconnect"
    );
    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
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
