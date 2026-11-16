//! Runtime coverage for the `response_delivery` span.
//!
//! The span is the only place a completed response reports what happened to
//! it: the request and correlation identifiers, whether the response streamed,
//! and the final outcome recorded after the terminator and completion hook have
//! run. A test that inspects the recording API only proves the fields exist;
//! only a captured close event proves they reach a subscriber. This module
//! drives a streamed response through the real forwarding path under a
//! capturing subscriber and asserts the exported fields. A second scenario
//! probes from inside the protocol hooks, proving the span is still in force
//! while the terminator is written and the completion hook runs, which is what
//! makes the recorded outcome describe both.

use std::{
    io,
    sync::{Arc, Mutex},
};

use tracing_subscriber::fmt::format::FmtSpan;

use super::terminator_tests::{
    CompletionProbe,
    TaggedFrame,
    forward_finished_stream,
    forward_finished_stream_with,
};
use crate::hooks::{ConnectionContext, ProtocolHooks};

/// Buffer the capturing subscriber formats span events into.
#[derive(Clone, Default)]
struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

impl CapturedOutput {
    /// Everything the subscriber has written so far, as text.
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

/// A completed stream records its outcome on the delivery span.
///
/// The subscriber guard is thread-local, so the test keeps the default
/// current-thread runtime: the forwarder must run on the thread that installed
/// the capture.
#[tokio::test]
async fn a_completed_stream_records_its_outcome_on_the_delivery_span() {
    let captured = CapturedOutput::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::CLOSE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let frames = forward_finished_stream(ProtocolHooks::default())
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

/// Hooks that record the span in force when `before_send` runs.
fn span_probing_hooks(observations: SpanObservations) -> ProtocolHooks<TaggedFrame, ()> {
    ProtocolHooks {
        before_send: Some(Box::new(
            move |_frame: &mut TaggedFrame, _ctx: &mut ConnectionContext| {
                observations.record_before_send(current_span_name());
            },
        )),
        ..ProtocolHooks::default()
    }
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
/// a close-event capture alone cannot.
#[tokio::test]
async fn the_delivery_span_covers_the_terminator_and_completion_hook() {
    let _guard = tracing::subscriber::set_default(tracing_subscriber::registry());

    let observations = SpanObservations::default();
    let frames = forward_finished_stream_with(
        span_probing_hooks(observations.clone()),
        Some(span_probing_completion(observations.clone())),
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
