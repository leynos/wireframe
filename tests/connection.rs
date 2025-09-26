//! Integration tests for connection actor queue helpers and multi-packet handling.
#![cfg(not(loom))]

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

/// Builder for protocol hook configurations used in connection tests.
struct TestHookBuilder {
    before_calls: Arc<AtomicUsize>,
    end_calls: Option<Arc<AtomicUsize>>,
}

impl TestHookBuilder {
    /// Create a builder tracking before-send invocations without an end hook.
    fn new() -> Self {
        Self {
            before_calls: Arc::new(AtomicUsize::new(0)),
            end_calls: None,
        }
    }

    /// Enable tracking for on-command-end hook invocations.
    fn with_end_hook(mut self) -> Self {
        self.end_calls = Some(Arc::new(AtomicUsize::new(0)));
        self
    }

    /// Construct protocol hooks that increment frames and record hook usage.
    fn build_hooks_with_increment<F>(
        self,
        increment: u8,
        stream_end_value: F,
    ) -> (ProtocolHooks<u8, ()>, Self)
    where
        F: Fn(&mut ConnectionContext) -> Option<u8> + Send + Sync + 'static,
    {
        let before_clone = self.before_calls.clone();
        let end_clone = self.end_calls.clone();

        let hooks = ProtocolHooks {
            before_send: Some(Box::new(
                move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                    before_clone.fetch_add(1, Ordering::SeqCst);
                    *frame += increment;
                },
            )),
            on_command_end: end_clone.as_ref().map(|end_calls| {
                let end_calls = end_calls.clone();
                Box::new(move |_ctx: &mut ConnectionContext| {
                    end_calls.fetch_add(1, Ordering::SeqCst);
                }) as Box<dyn FnMut(&mut ConnectionContext) + Send + 'static>
            }),
            stream_end: Some(Box::new(stream_end_value)),
            ..ProtocolHooks::<u8, ()>::default()
        };

        (hooks, self)
    }

    /// Snapshot the recorded hook counts for verification.
    fn get_counts(&self) -> HookCounts {
        HookCounts::from_counters(&self.before_calls, self.end_calls.as_ref())
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
    let (hooks, hook_builder) = TestHookBuilder::new().build_hooks_with_increment(1, |_| None);

    let mut harness =
        ActorHarness::new_with_state(hooks, false, false).expect("failed to create harness");
    harness.process_multi_packet(Some(5));

    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
    assert_frame_processed(
        &harness.out,
        &[6],
        HookCounts { before: 1, end: 0 },
        hook_builder.get_counts(),
    );
}

#[test]
fn process_multi_packet_none_emits_end_frame() {
    let (hooks, hook_builder) = TestHookBuilder::new()
        .with_end_hook()
        .build_hooks_with_increment(2, |_| Some(9));

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
        hook_builder.get_counts(),
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
    let terminator_value = terminator;
    let (hooks, hook_builder) = TestHookBuilder::new()
        .with_end_hook()
        .build_hooks_with_increment(1, move |_ctx| terminator_value);

    let mut harness =
        ActorHarness::new_with_state(hooks, false, true).expect("failed to create harness");
    let (_tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx));

    harness.handle_multi_packet_closed();

    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
    assert!(
        !harness.has_multi_queue(),
        "multi-packet channel should be cleared",
    );
    assert_frame_processed(
        &harness.out,
        &expected_output,
        HookCounts {
            before: expected_before,
            end: 1,
        },
        hook_builder.get_counts(),
    );
}

#[test]
fn try_opportunistic_drain_forwards_frame() {
    let (hooks, hook_builder) = TestHookBuilder::new().build_hooks_with_increment(1, |_| None);

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
        hook_builder.get_counts(),
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
