//! Runtime coverage for the `response_delivery` span.
//!
//! The span is the only place a completed response reports what happened to
//! it: the request and correlation identifiers, whether the response streamed,
//! and the final outcome recorded after the terminator and completion hook have
//! run. A test that inspects the recording API only proves the fields exist;
//! only a captured close event proves they reach a subscriber. This module
//! drives a streamed response through the real forwarding path under a
//! capturing subscriber and asserts the exported fields.

use std::{
    io,
    sync::{Arc, Mutex},
};

use tracing_subscriber::fmt::format::FmtSpan;

use super::terminator_tests::forward_finished_stream;
use crate::hooks::ProtocolHooks;

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
