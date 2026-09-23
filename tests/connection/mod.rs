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
    connection::test_support::{ActorHarness, ActorStateHarness, poll_queue_next},
    hooks::{ConnectionContext, ProtocolHooks},
};
use wireframe_testing::{LoggerHandle, TestResult, logger};

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
    ) -> ProtocolHooks<u8, wireframe::NoProtocolError> {
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
            ..ProtocolHooks::<u8, wireframe::NoProtocolError>::default()
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
    let expected_correlation = format!("correlation_id={expected_correlation:?}");
    let mut found = false;
    while let Some(record) = logger.pop() {
        let message = record.args().to_string();
        if !message.contains("multi-packet stream closed") {
            continue;
        }
        if !message.contains(&expected_correlation) {
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

mod queue_and_shutdown;
