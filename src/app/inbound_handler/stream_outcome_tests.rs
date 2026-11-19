//! Accounting for body outcomes that only one branch can report.
//!
//! A streaming body's fate is counted where the branch that decides it runs.
//! Two branches have no later chance to report the body: the final chunk is the
//! last thing a body sends, so nothing follows it, and a handler that answers
//! before its terminal failure is delivered leaves no completed delivery to
//! report against. Counting either one is invisible in the connection's
//! behaviour, so both are proved here by reading the recorded outcomes.

use std::{future, io, sync::Arc, time::Duration};

use bytes::Bytes;
use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use rstest::{fixture, rstest};

use super::{
    super::{
        stream_active::{BodyIdentity, HandlerFuture, PendingEvent},
        stream_terminal::{HandlerWait, TerminalFailure, deliver_then_await},
    },
    ConnectionStreams,
};
use crate::{
    app::{
        StreamingHandler,
        inbound_handler::stream_handles::{ChunkBudget, FirstChunk},
    },
    metrics::{STREAMING_BODY_OUTCOMES, StreamingBodyOutcome},
    request::{QueuedBytes, RequestBodyStream, RequestParts, accounted_body_channel},
};

/// One metric sample as the debugging recorder reports it.
type Sample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// The final chunk of a single-frame streaming body.
#[fixture]
fn final_chunk() -> FirstChunk {
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
fn generous_budget() -> ChunkBudget {
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
fn dropping_handler() -> StreamingHandler {
    Arc::new(|_parts: RequestParts, body: RequestBodyStream| {
        drop(body);
        Box::pin(future::pending())
    })
}

/// A truncated body failure for the outcome scenario's message.
fn truncated_failure() -> TerminalFailure {
    TerminalFailure {
        identity: BodyIdentity {
            message_id: 7,
            correlation_id: None,
        },
        error: io::Error::new(io::ErrorKind::UnexpectedEof, "truncated body"),
    }
}

/// Run `scenario` with a private recorder and count the `outcome` it recorded.
///
/// The recorder is thread-local and the scenario runs on this thread, so the
/// count covers exactly this scenario instead of every test sharing the
/// process-wide counter.
fn counted<T>(outcome: StreamingBodyOutcome, scenario: impl FnOnce() -> T) -> (T, u64) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let value = ::metrics::with_local_recorder(&recorder, scenario);
    let count = snapshotter
        .snapshot()
        .into_vec()
        .iter()
        .filter(|sample| counts_outcome(sample, outcome))
        .map(|(_, _, _, recorded)| match recorded {
            DebugValue::Counter(count) => *count,
            _ => 0,
        })
        .sum();
    (value, count)
}

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
/// The recorder has to enclose the await, and the scenario only ever waits on
/// timers that the paused clock advances at once, so the runtime is built here
/// rather than inherited from a test attribute. Building can fail, so the error
/// is returned for the calling test to report rather than panicking here.
fn paused_block_on<F: future::Future>(future: F) -> io::Result<F::Output> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    Ok(runtime.block_on(future))
}

/// A refused final chunk leaves the body abandoned rather than uncounted.
///
/// A final chunk is the last thing a body sends, so no later chunk can report
/// what became of it. A handler that had already dropped its stream therefore
/// has to be counted here, as the parked path counts the same case.
#[rstest]
#[test]
fn a_refused_final_chunk_is_counted_as_abandoned(
    final_chunk: FirstChunk,
    generous_budget: ChunkBudget,
) {
    let (settled, abandoned) = counted(StreamingBodyOutcome::Abandoned, || {
        paused_block_on(async {
            let mut streams = ConnectionStreams::new(QueuedBytes::new());
            streams.start(final_chunk, &dropping_handler());
            let event = streams
                .drive_pending(Duration::from_secs(1), generous_budget)
                .await;
            let PendingEvent::Send(result) = event else {
                panic!("the admission must settle against a closed body");
            };
            assert!(result.is_err(), "the handler's stream is already gone");
            streams.complete_send(result);
            assert!(
                streams
                    .active
                    .as_ref()
                    .is_some_and(|active| active.sender.is_none()),
                "a finished final chunk leaves no sender behind"
            );
        })
    });

    settled.expect("the admission settles against the closed body");
    assert_eq!(abandoned, 1, "the refused final chunk is counted once");
}

/// A handler that answers first leaves its body abandoned, counted once.
///
/// The failure needs a channel slot that only this handler could free, so the
/// delivery cannot complete before the reply does. Nothing else reports the
/// body in that case: the delivery branch never completed, and the handler's
/// reply says nothing about the body it abandoned.
#[rstest]
#[test]
fn a_handler_answering_first_is_counted_as_abandoned() {
    let (observed, abandoned) = counted(StreamingBodyOutcome::Abandoned, || {
        paused_block_on(async {
            let (sender, _body) = accounted_body_channel(1, QueuedBytes::new());
            sender
                .clone()
                .send_chunk(Bytes::from_static(b"unread"), 0, usize::MAX)
                .await
                .expect("an empty channel admits one chunk");
            let mut handler: HandlerFuture =
                Box::pin(future::ready(Some(Bytes::from_static(b"reply"))));
            deliver_then_await(
                Duration::from_millis(50),
                truncated_failure(),
                sender,
                &mut handler,
            )
            .await
        })
    });

    let wait = observed.expect("the scenario runs on a paused runtime");
    assert!(
        matches!(wait, HandlerWait::Replied(Some(_))),
        "the reply settles before the full channel can accept the failure"
    );
    assert_eq!(abandoned, 1, "the abandoned body is counted once");
}
