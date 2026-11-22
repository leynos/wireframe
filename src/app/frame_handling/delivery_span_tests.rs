//! Runtime coverage for the `response_delivery` span.
//!
//! The span is the only place a completed response reports what happened to
//! it: the request and correlation identifiers, whether the response streamed,
//! and the final outcome recorded after the terminator and completion hook have
//! run. A test that inspects the recording API only proves the fields exist;
//! only a span reaching a subscriber proves they are exported. This module
//! drives a streamed response through the real forwarding path under a
//! capturing subscriber and asserts the exported fields. A second scenario
//! probes from inside the protocol hooks, proving the span is still in force
//! while the terminator is written and the completion hook runs, which is what
//! makes the recorded outcome describe both.
//!
//! Two pieces of harness care are needed. Each scenario installs its subscriber
//! with `set_default`, scoped to the test thread, because the process-wide slot
//! belongs to `tracing_test`: installing a global default here would fail and
//! poison `tracing_test` for every other test in the binary. Interest caching is
//! then kept permissive by [`keep_interest_cache_permissive`], without which
//! the delivery span is silently never created.

use std::{
    io,
    sync::{Arc, Mutex, OnceLock},
};

use tracing_subscriber::fmt::format::FmtSpan;

use super::terminator_tests::{
    BeforeSendProbe,
    CompletionProbe,
    TaggedFrame,
    forward_finished_stream,
    forward_finished_stream_with,
};

/// Keep the process-wide callsite interest cache permissive.
///
/// `tracing-core` recomputes every callsite's cached interest whenever a
/// dispatcher is registered. With at most one registered dispatcher it consults
/// the registering thread's default dispatcher instead, so a thread with no
/// default — every test thread before its first installation — caches "never"
/// for every callsite. The delivery span would then never be created, and its
/// hooks would run outside any span. Keeping two permissive registries alive
/// switches the cache to the mode that combines the registered dispatchers,
/// which is stable across threads and registration order. They are kept in a
/// `OnceLock` rather than dropped because a dropped dispatcher is pruned from
/// the registry on the next registration.
fn keep_interest_cache_permissive() {
    static REGISTRIES: OnceLock<Vec<tracing::Dispatch>> = OnceLock::new();
    let kept = REGISTRIES.get_or_init(|| {
        (0..2)
            .map(|_| tracing::Dispatch::new(tracing_subscriber::registry()))
            .collect()
    });
    debug_assert_eq!(
        kept.len(),
        2,
        "both permissive registries must stay registered"
    );
}

/// Buffer the capturing subscriber formats span events into.
#[derive(Clone, Default)]
struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

impl CapturedOutput {
    /// Everything captured so far, as text.
    ///
    /// The subscriber formats span events, so a lossy conversion is enough:
    /// the assertions look for ASCII field names and values.
    fn text(&self) -> String {
        self.0
            .lock()
            .map(|buffer| String::from_utf8_lossy(&buffer).into_owned())
            .unwrap_or_default()
    }
}

impl io::Write for CapturedOutput {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut buffer = self
            .0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        buffer.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedOutput {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer { self.clone() }
}

/// Install a capturing subscriber for the current test thread.
///
/// Span close events are formatted so the delivery span's recorded fields reach
/// the capture; the guard must stay alive for the whole scenario. The interest
/// rebuild recomputes the cache against the now-current subscriber.
fn capture_spans(captured: &CapturedOutput) -> tracing::subscriber::DefaultGuard {
    keep_interest_cache_permissive();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::CLOSE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    guard
}

/// A completed stream records its outcome on the delivery span.
///
/// The capture is scoped to the test thread, so the test keeps the default
/// current-thread runtime: the forwarder must run on the thread that installed
/// it.
#[tokio::test]
async fn a_completed_stream_records_its_outcome_on_the_delivery_span() {
    let captured = CapturedOutput::default();
    let _capture = capture_spans(&captured);

    let frames = forward_finished_stream()
        .await
        .expect("the finished stream must be forwarded");

    assert_eq!(
        frames.len(),
        1,
        "the finished stream writes its protocol terminator"
    );

    let output = captured.text();
    assert!(
        output.contains("response_delivery"),
        "the delivery span must reach the subscriber; captured: {output}"
    );
    for field in [
        "request_id=7",
        "correlation_id=Some(3)",
        "streaming=true",
        "outcome=Completed",
    ] {
        assert!(
            output.contains(field),
            "the delivery span must report {field}; captured: {output}"
        );
    }
}

/// Marker recorded when a probe ran outside every span.
const NO_SPAN: &str = "<no span>";

/// Name of the span in force where a probe runs.
fn current_span_name() -> &'static str {
    tracing::Span::current()
        .metadata()
        .map_or(NO_SPAN, |metadata| metadata.name())
}

/// What the protocol hooks observed about the span in force.
#[derive(Clone, Default)]
struct SpanObservations(Arc<Mutex<SpanRecords>>);

/// Span names each hook recorded, in call order.
#[derive(Clone, Default)]
struct SpanRecords {
    before_send: Vec<&'static str>,
    command_end: Vec<&'static str>,
}

impl SpanObservations {
    /// Note the span a `before_send` probe ran in.
    fn record_before_send(&self, span: &'static str) { self.lock().before_send.push(span); }

    /// Note the span a completion-hook probe ran in.
    fn record_command_end(&self, span: &'static str) { self.lock().command_end.push(span); }

    /// Everything recorded so far.
    fn records(&self) -> SpanRecords { self.lock().clone() }

    /// Take the short lock the tests use for a single push or read.
    ///
    /// A probe runs from inside the forwarding path, so a poisoned lock is
    /// recovered rather than propagated: the recorded names are still the
    /// evidence the test needs, and panicking here would replace the
    /// assertion failure with a lock error.
    fn lock(&self) -> std::sync::MutexGuard<'_, SpanRecords> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Probe that records the span in force when `before_send` runs.
fn span_probing_before_send(observations: SpanObservations) -> BeforeSendProbe {
    Arc::new(move |_frame: &mut TaggedFrame| {
        observations.record_before_send(current_span_name());
    })
}

/// Completion probe that records the span in force when the hook runs.
fn span_probing_completion(observations: SpanObservations) -> CompletionProbe {
    Arc::new(move || observations.record_command_end(current_span_name()))
}

/// The delivery span is in force for the terminator write and completion hook.
///
/// The span records its outcome after the instrumented future resolves, so the
/// recording only describes the terminator and the hook if both run while the
/// span is current. Probing from inside each hook proves that directly, which
/// a close-event capture alone cannot. The subscriber is installed only to
/// enable spans; the probes read the span directly.
#[tokio::test]
async fn the_delivery_span_covers_the_terminator_and_completion_hook() {
    let _capture = capture_spans(&CapturedOutput::default());

    let observations = SpanObservations::default();
    let frames = forward_finished_stream_with(
        Some(span_probing_completion(observations.clone())),
        Some(span_probing_before_send(observations.clone())),
    )
    .await
    .expect("the finished stream must be forwarded");

    assert_eq!(
        frames.len(),
        1,
        "the finished stream writes its protocol terminator"
    );

    let records = observations.records();
    assert_eq!(
        records.before_send,
        vec!["response_delivery"],
        "the terminator's before_send hook must run inside the delivery span"
    );
    assert_eq!(
        records.command_end,
        vec!["response_delivery"],
        "the completion hook must run inside the delivery span"
    );
}
