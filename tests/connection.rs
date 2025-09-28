//! Integration tests for connection actor queue helpers and multi-packet handling.
#![cfg(not(loom))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rstest::{fixture, rstest};
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

/// Shared counters for protocol hook configurations used in connection tests.
#[derive(Clone)]
struct HookCounters {
    before_calls: Arc<AtomicUsize>,
    end_calls: Arc<AtomicUsize>,
}

impl HookCounters {
    /// Create counters tracking before-send and on-command-end hook invocations.
    fn new() -> Self {
        Self {
            before_calls: Arc::new(AtomicUsize::new(0)),
            end_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Construct protocol hooks that increment frames and record hook usage.
    fn build_hooks_with_increment(
        &self,
        increment: u8,
        stream_end_value: impl Fn(&mut ConnectionContext) -> Option<u8> + Send + Sync + 'static,
    ) -> ProtocolHooks<u8, ()> {
        let before_clone = Arc::clone(&self.before_calls);
        let end_clone = Arc::clone(&self.end_calls);

        ProtocolHooks {
            before_send: Some(Box::new(
                move |frame: &mut u8, _ctx: &mut ConnectionContext| {
                    before_clone.fetch_add(1, Ordering::SeqCst);
                    *frame += increment;
                },
            )),
            on_command_end: Some(Box::new(move |_ctx: &mut ConnectionContext| {
                end_clone.fetch_add(1, Ordering::SeqCst);
            })
                as Box<dyn FnMut(&mut ConnectionContext) + Send + 'static>),
            stream_end: Some(Box::new(stream_end_value)),
            ..ProtocolHooks::<u8, ()>::default()
        }
    }

    /// Snapshot the recorded hook counts for verification.
    fn get_counts(&self) -> HookCounts {
        HookCounts::from_counters(&self.before_calls, Some(&self.end_calls))
    }
}

type StreamEndHook = Box<dyn Fn(&mut ConnectionContext) -> Option<u8> + Send + Sync + 'static>;

/// Configuration for building connection actor harnesses.
#[derive(Default)]
struct HarnessConfig {
    has_response: bool,
    has_multi_packet: bool,
    increment: u8,
    stream_end_value: Option<StreamEndHook>,
}

impl HarnessConfig {
    /// Create a harness configuration with the default increment.
    fn new() -> Self {
        Self {
            increment: 1,
            ..Self::default()
        }
    }

    /// Enable response tracking for the harness under construction.
    #[expect(
        dead_code,
        reason = "Harness builders retain response toggles for future scenarios."
    )]
    fn with_response(mut self) -> Self {
        self.has_response = true;
        self
    }

    /// Enable multi-packet support for the harness under construction.
    fn with_multi_packet(mut self) -> Self {
        self.has_multi_packet = true;
        self
    }

    /// Override the amount added to each forwarded frame.
    fn with_increment(mut self, increment: u8) -> Self {
        self.increment = increment;
        self
    }

    /// Provide a stream-end hook for the harness under construction.
    fn with_stream_end<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut ConnectionContext) -> Option<u8> + Send + Sync + 'static,
    {
        self.stream_end_value = Some(Box::new(f));
        self
    }
}

#[derive(Clone)]
struct HarnessFactory {
    counters: HookCounters,
}

impl HarnessFactory {
    /// Build a connection actor harness with shared hook counters.
    fn create(&self, config: HarnessConfig) -> ActorHarness {
        let HarnessConfig {
            has_response,
            has_multi_packet,
            increment,
            stream_end_value,
        } = config;
        let stream_end_fn =
            stream_end_value.unwrap_or_else(|| Box::new(|_: &mut ConnectionContext| None));
        let hooks = self
            .counters
            .build_hooks_with_increment(increment, stream_end_fn);
        ActorHarness::new_with_state(hooks, has_response, has_multi_packet)
            .expect("failed to create harness")
    }

    /// Read the accumulated hook counters for the most recent harness.
    fn counts(&self) -> HookCounts { self.counters.get_counts() }
}

#[fixture]
fn hook_counters() -> HookCounters {
    let counters = HookCounters::new();
    debug_assert_eq!(counters.get_counts(), HookCounts { before: 0, end: 0 });
    counters
}

#[fixture]
fn harness_factory(hook_counters: HookCounters) -> HarnessFactory {
    HarnessFactory {
        counters: hook_counters,
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

/// Helper to verify common multi-packet processing assertions.
fn assert_multi_packet_processing_result(
    harness: &ActorHarness,
    harness_factory: &HarnessFactory,
    expected_output: &[u8],
    expected_counts: HookCounts,
) {
    let snapshot = harness.snapshot();
    assert!(snapshot.is_active && !snapshot.is_shutting_down && !snapshot.is_done);
    assert_frame_processed(
        &harness.out,
        expected_output,
        expected_counts,
        harness_factory.counts(),
    );
}

#[rstest]
fn process_multi_packet_forwards_frame(harness_factory: HarnessFactory) {
    let mut harness = harness_factory.create(HarnessConfig::new());
    harness.process_multi_packet(Some(5));

    assert_multi_packet_processing_result(
        &harness,
        &harness_factory,
        &[6],
        HookCounts { before: 1, end: 0 },
    );
}

#[rstest]
fn process_multi_packet_none_emits_end_frame(harness_factory: HarnessFactory) {
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_increment(2)
            .with_stream_end(|_| Some(9)),
    );
    let (_tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx));

    harness.process_multi_packet(None);

    assert_multi_packet_processing_result(
        &harness,
        &harness_factory,
        &[11],
        HookCounts { before: 1, end: 1 },
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
    harness_factory: HarnessFactory,
    terminator: Option<u8>,
    expected_output: Vec<u8>,
    expected_before: usize,
) {
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(move |_| terminator),
    );
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
        harness_factory.counts(),
    );
}

#[rstest]
fn try_opportunistic_drain_forwards_frame(harness_factory: HarnessFactory) {
    let mut harness = harness_factory.create(HarnessConfig::new());
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
        harness_factory.counts(),
    );
}

#[rstest]
fn try_opportunistic_drain_multi_disconnect_emits_terminator(harness_factory: HarnessFactory) {
    let mut harness = harness_factory.create(
        HarnessConfig::new()
            .with_multi_packet()
            .with_stream_end(|_| Some(5)),
    );
    let (tx, rx) = mpsc::channel(1);
    harness.set_multi_queue(Some(rx));
    drop(tx);

    let drained = harness.try_drain_multi();

    assert!(!drained, "disconnect should not report a drained frame",);
    assert!(
        !harness.has_multi_queue(),
        "multi-packet queue should be cleared after disconnect",
    );
    assert_frame_processed(
        &harness.out,
        &[6],
        HookCounts { before: 1, end: 1 },
        harness_factory.counts(),
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
        "queue should be cleared after disconnect",
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
