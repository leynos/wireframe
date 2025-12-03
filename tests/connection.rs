//! Integration tests for connection actor queue helpers and multi-packet handling.
#![cfg(not(loom))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use log::Level;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::sync::mpsc;
use wireframe::{
    ProtocolHooks,
    connection::test_support::{ActorHarness, ActorStateHarness, poll_queue_next},
    hooks::ConnectionContext,
};
use wireframe_testing::{LoggerHandle, logger};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

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
    fn create(
        &self,
        config: HarnessConfig,
    ) -> Result<ActorHarness, wireframe::push::PushConfigError> {
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

fn assert_reason_logged(
    logger: &mut LoggerHandle,
    expected_level: Level,
    expected_reason: &str,
    expected_correlation: Option<u64>,
) {
    let mut found = false;
    while let Some(record) = logger.pop() {
        let message = record.args().to_string();
        if !message.contains("multi-packet stream closed") {
            continue;
        }
        assert_eq!(
            record.level(),
            expected_level,
            "unexpected log level for closure: message={message}",
        );
        assert!(
            message.contains(&format!("reason={expected_reason}")),
            "closure log missing reason: message={message}",
        );
        assert!(
            message.contains(&format!("correlation_id={expected_correlation:?}")),
            "closure log missing correlation: message={message}",
        );
        found = true;
        break;
    }
    assert!(found, "multi-packet closure log not found");
}

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
    harness.set_multi_queue(Some(rx));

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
    harness.set_multi_queue(Some(rx));

    harness.handle_multi_packet_closed();

    let snapshot = harness.snapshot();
    assert!(snapshot.is_active, "connection should be active");
    assert!(
        !snapshot.is_shutting_down,
        "connection should not be shutting down"
    );
    assert!(!snapshot.is_done, "connection should not be done");
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
        .set_multi_packet_with_correlation(Some(rx), Some(11));
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
        .set_multi_packet_with_correlation(Some(rx), Some(12));
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
        .set_multi_packet_with_correlation(Some(rx), Some(13));
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
    harness.set_multi_queue(Some(rx));
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
    assert!(snapshot.is_active, "connection should be active");
    assert!(
        !snapshot.is_shutting_down,
        "connection should not be shutting down"
    );
    assert!(!snapshot.is_done, "connection should not be done");
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
