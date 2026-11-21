//! Shared scaffolding for the streaming-body outcome tests.
//!
//! Every streaming body ends in exactly one recorded outcome, but the branch
//! that records it lives in a different module for each case: the slot for a
//! final chunk or an abandoned exchange, the terminal helpers for a delivered
//! failure or a bounded wait, and the ingress and parked paths for a missing
//! handler or a pre-filled body. The counting helpers and the message shapes
//! are shared here so each test reads as one branch and its expected count.

use std::{future, io, sync::Arc, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use rstest::fixture;

use crate::{
    app::{
        StreamingHandler,
        inbound_handler::{
            stream_active::{BodyIdentity, HandlerFuture},
            stream_handles::{ChunkBudget, FirstChunk},
            stream_terminal::{HandlerWait, TerminalFailure},
        },
    },
    metrics::{STREAMING_BODY_OUTCOMES, StreamingBodyOutcome},
    request::{AccountedBodySender, RequestBodyStream, RequestParts},
};

/// One metric sample as the debugging recorder reports it.
pub(super) type Sample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// The final chunk of a single-frame streaming body.
#[fixture]
pub(super) fn final_chunk() -> FirstChunk {
    FirstChunk {
        key: None,
        assembly: None,
        parts: RequestParts::new(7, None, Vec::new()),
        chunk: Bytes::from_static(b"last"),
        is_last: true,
    }
}

/// Admission budget that never refuses a chunk on size.
#[fixture]
pub(super) fn generous_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: usize::MAX,
    }
}

/// Handler that drops its body before reading anything and never answers.
///
/// This is what a handler which never wanted the body looks like: the channel
/// closes as the handler is created, so every chunk is refused. Staying
/// pending keeps the drive from racing the refusal: if the handler resolved
/// too, either branch could win, and the refusal is the branch under test.
pub(super) fn dropping_handler() -> StreamingHandler {
    Arc::new(|_parts: RequestParts, body: RequestBodyStream| {
        drop(body);
        Box::pin(future::pending())
    })
}

/// A truncated body failure for the outcome scenarios' message.
pub(super) fn truncated_failure() -> TerminalFailure {
    TerminalFailure {
        identity: identity(),
        error: io::Error::new(io::ErrorKind::UnexpectedEof, "truncated body"),
    }
}

/// Identity of the message the outcome scenarios serve.
pub(super) const fn identity() -> BodyIdentity {
    BodyIdentity {
        message_id: 7,
        correlation_id: None,
    }
}

/// Handler future that reads its body to the end and answers with what it saw.
///
/// The reply is the body's own bytes when the body ended cleanly, or `failed`
/// when it failed. One assertion therefore distinguishes a whole body from a
/// truncated one, and a handler built this way reads the channel like any real
/// streaming route would.
pub(super) fn reading_handler(mut body: RequestBodyStream) -> HandlerFuture {
    Box::pin(async move {
        let mut received = Vec::new();
        while let Some(chunk) = body.next().await {
            match chunk {
                Ok(bytes) => received.extend_from_slice(&bytes),
                Err(_) => return Some(Bytes::from_static(b"failed")),
            }
        }
        Some(Bytes::from(received))
    })
}

/// Route handler that reads its body through [`reading_handler`].
///
/// The slot starts a body from a route handler rather than from a future, so a
/// scenario that goes through
/// [`ConnectionStreams::start`](super::stream_slot::ConnectionStreams::start) needs one of these to
/// observe what the handler received.
pub(super) fn reading_route() -> StreamingHandler {
    Arc::new(|_parts: RequestParts, body: RequestBodyStream| reading_handler(body))
}

/// The outcome counts one scenario recorded.
pub(super) struct Counts(Vec<Sample>);

impl Counts {
    /// Return how many times `outcome` was recorded.
    pub(super) fn count(&self, outcome: StreamingBodyOutcome) -> u64 {
        self.0
            .iter()
            .filter(|sample| counts_outcome(sample, outcome))
            .map(|(_, _, _, recorded)| match recorded {
                DebugValue::Counter(count) => *count,
                _ => 0,
            })
            .sum()
    }
}

/// Run `scenario` with a private recorder and return every outcome it recorded.
///
/// The recorder is thread-local and the scenario runs on this thread, so the
/// counts cover exactly this scenario instead of every test sharing the
/// process-wide counter.
pub(super) fn recorded<T>(scenario: impl FnOnce() -> T) -> (T, Counts) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let value = ::metrics::with_local_recorder(&recorder, scenario);
    let counts = Counts(snapshotter.snapshot().into_vec());
    (value, counts)
}

/// Run `scenario` with a private recorder and count the `outcome` it recorded.
pub(super) fn counted<T>(outcome: StreamingBodyOutcome, scenario: impl FnOnce() -> T) -> (T, u64) {
    let (value, counts) = recorded(scenario);
    (value, counts.count(outcome))
}

/// What one exchange should have settled with and counted.
///
/// Grouped rather than passed separately because the three travel together:
/// every exchange settles with one reply and records one outcome, and the
/// reason is what makes a failure readable.
pub(super) struct Expectation {
    /// Reply the exchange should settle with.
    pub(super) reply: HandlerWait,
    /// The one outcome the exchange should have recorded.
    pub(super) recorded: StreamingBodyOutcome,
    /// What the exchange was supposed to establish, for failure messages.
    pub(super) why: &'static str,
}

/// Assert the reply an exchange settled with and the outcome it recorded.
///
/// A body records exactly one outcome, so the whole vocabulary is checked: the
/// expected outcome was counted once and every other one never. Asserting only
/// the expected pair would let a branch that also recorded a second outcome
/// pass.
pub(super) fn assert_exchange(
    settled: io::Result<HandlerWait>,
    counts: &Counts,
    expected: &Expectation,
) {
    let reply = match settled {
        Ok(wait) => wait,
        Err(error) => panic!("the exchange must run on a paused runtime: {error}"),
    };
    let why = expected.why;
    assert_eq!(reply, expected.reply, "{why}");
    for outcome in OUTCOMES {
        let count = u64::from(outcome == expected.recorded);
        assert_eq!(
            counts.count(outcome),
            count,
            "{} recorded {count} times: {why}",
            outcome.as_str()
        );
    }
}

/// Every outcome a streaming body can record.
///
/// Listed here rather than read from the enum: the counter's vocabulary is
/// production code, and a test-local list is what makes an unasserted outcome
/// visible when one is added.
const OUTCOMES: [StreamingBodyOutcome; 6] = [
    StreamingBodyOutcome::Delivered,
    StreamingBodyOutcome::Terminal,
    StreamingBodyOutcome::Abandoned,
    StreamingBodyOutcome::AdmissionTimeout,
    StreamingBodyOutcome::DeliveryTimeout,
    StreamingBodyOutcome::NoHandler,
];

/// Return whether `sample` is the counter for `outcome`.
fn counts_outcome(sample: &Sample, outcome: StreamingBodyOutcome) -> bool {
    let (key, ..) = sample;
    key.key().name() == STREAMING_BODY_OUTCOMES
        && key
            .key()
            .labels()
            .any(|label| label.key() == "outcome" && label.value() == outcome.as_str())
}

/// Run `future` on a paused current-thread runtime.
///
/// The recorder has to enclose the await, and the scenarios only ever wait on
/// timers that the paused clock advances at once, so the runtime is built here
/// rather than inherited from a test attribute. Building can fail, so the error
/// is returned for the calling test to report rather than panicking here.
pub(super) fn paused_block_on<F: future::Future>(future: F) -> io::Result<F::Output> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    Ok(runtime.block_on(future))
}

/// A wait short enough that a stalled exchange hands control back quickly.
pub(super) const fn wait() -> Duration { Duration::from_millis(50) }

/// Fill a body channel that holds one value, so the next send must wait.
///
/// The queue is empty and its capacity is not reached, so the send cannot
/// fail; the refusal is discarded only because the call has no other outcome.
pub(super) async fn queue_one_chunk(sender: &AccountedBodySender) {
    let _refused_if_reached = sender
        .clone()
        .send_chunk(Bytes::from_static(b"chunk"), 0, usize::MAX)
        .await;
}
