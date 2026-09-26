//! Shared counters, harness configuration, and assertions for connection tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use log::Level;
use rstest::fixture;
use wireframe::{
    connection::test_support::ActorHarness,
    hooks::{ConnectionContext, ProtocolHooks},
};
use wireframe_testing::LoggerHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HookCounts {
    pub(crate) before: usize,
    pub(crate) end: usize,
}

impl HookCounts {
    /// Snapshot hook counters from shared state.
    pub(crate) fn from_counters(before: &Arc<AtomicUsize>, end: Option<&Arc<AtomicUsize>>) -> Self {
        let end = end.map_or(0, |counter| counter.load(Ordering::SeqCst));
        Self {
            before: before.load(Ordering::SeqCst),
            end,
        }
    }
}

/// Shared counters for protocol hook configurations used in connection tests.
#[derive(Clone)]
pub(crate) struct HookCounters {
    before_calls: Arc<AtomicUsize>,
    end_calls: Arc<AtomicUsize>,
}

impl HookCounters {
    /// Create counters tracking before-send and on-command-end hook invocations.
    pub(crate) fn new() -> Self {
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
    pub(crate) fn get_counts(&self) -> HookCounts {
        HookCounts::from_counters(&self.before_calls, Some(&self.end_calls))
    }
}

type StreamEndHook = Box<dyn Fn(&mut ConnectionContext) -> Option<u8> + Send + Sync + 'static>;

/// Configuration for building connection actor harnesses.
#[derive(Default)]
pub(crate) struct HarnessConfig {
    has_response: bool,
    has_multi_packet: bool,
    increment: u8,
    stream_end_value: Option<StreamEndHook>,
}

impl HarnessConfig {
    /// Create a harness configuration with the default increment.
    pub(crate) fn new() -> Self {
        Self {
            increment: 1,
            ..Self::default()
        }
    }

    /// Enable multi-packet support for the harness under construction.
    pub(crate) fn with_multi_packet(mut self) -> Self {
        self.has_multi_packet = true;
        self
    }

    /// Override the amount added to each forwarded frame.
    pub(crate) fn with_increment(mut self, increment: u8) -> Self {
        self.increment = increment;
        self
    }

    /// Provide a stream-end hook for the harness under construction.
    pub(crate) fn with_stream_end<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut ConnectionContext) -> Option<u8> + Send + Sync + 'static,
    {
        self.stream_end_value = Some(Box::new(f));
        self
    }
}

#[derive(Clone)]
pub(crate) struct HarnessFactory {
    counters: HookCounters,
}

impl HarnessFactory {
    /// Build a connection actor harness with shared hook counters.
    pub(crate) fn create(
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
    pub(crate) fn counts(&self) -> HookCounts { self.counters.get_counts() }
}

#[fixture]
pub(crate) fn hook_counters() -> HookCounters {
    let counters = HookCounters::new();
    debug_assert_eq!(counters.get_counts(), HookCounts { before: 0, end: 0 });
    counters
}

#[fixture]
pub(crate) fn harness_factory(hook_counters: HookCounters) -> HarnessFactory {
    HarnessFactory {
        counters: hook_counters,
    }
}

pub(crate) fn assert_frame_processed(
    out: &[u8],
    expected: &[u8],
    expected_counts: HookCounts,
    actual_counts: HookCounts,
) {
    assert_eq!(out, expected, "frames should match expected output");
    assert_eq!(actual_counts, expected_counts, "hook counts should match");
}

/// Helper to verify common multi-packet processing assertions.
pub(crate) fn assert_multi_packet_processing_result(
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

pub(crate) fn assert_reason_logged(
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
