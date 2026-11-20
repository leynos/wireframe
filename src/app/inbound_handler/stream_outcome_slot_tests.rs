//! Accounting for the body outcomes the streaming slot itself decides.
//!
//! A streaming body's fate is counted where the branch that decides it runs.
//! Three of those branches are the slot's: a final chunk that reached its
//! handler, a final chunk the handler had already refused, and an exchange that
//! ended with its body incomplete. None of them has a later chance to report
//! the body — a final chunk is the last thing a body sends, and an exchange
//! that is finishing or aborting owns the only remaining state — so each is
//! proved here by reading the recorded outcome.

use std::time::Duration;

use bytes::Bytes;
use rstest::rstest;
use tracing_test::traced_test;

use super::{
    super::stream_outcome_support::{
        counted,
        dropping_handler,
        final_chunk,
        generous_budget,
        paused_block_on,
        reading_route,
        recorded,
        wait,
    },
    ConnectionStreams,
    record_unfinished,
};
use crate::{
    app::inbound_handler::{
        stream_active::{PendingEvent, PendingSend},
        stream_handles::{ChunkBudget, FirstChunk},
    },
    metrics::StreamingBodyOutcome,
    request::{QueuedBytes, accounted_body_channel},
};

/// A refused final chunk leaves the body abandoned rather than uncounted.
///
/// A final chunk is the last thing a body sends, so no later chunk can report
/// what became of it. A handler that had already dropped its stream therefore
/// has to be counted here, as the parked path counts the same case, and the
/// diagnostic event recorded beside the counter is asserted here too: it is
/// the only other trace the body leaves.
#[traced_test]
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
    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .any(|line| {
                line.contains("final request-body chunk refused") && line.contains("abandoned_body")
            })
            .then_some(())
            .ok_or_else(|| "the refused final chunk is not reported".to_owned())
    });
}

/// A final chunk the handler accepted is counted as delivered, not abandoned.
///
/// `complete_send` is the only place that learns a final chunk arrived, so an
/// uncounted success here would make a healthy body look like one that never
/// finished.
#[rstest]
#[test]
fn a_delivered_final_chunk_is_counted_as_delivered(
    final_chunk: FirstChunk,
    generous_budget: ChunkBudget,
) {
    let (settled, counts) = recorded(|| {
        paused_block_on(async {
            let mut streams = ConnectionStreams::new(QueuedBytes::new());
            streams.start(final_chunk, &reading_route());
            let event = streams
                .drive_pending(Duration::from_secs(1), generous_budget)
                .await;
            let PendingEvent::Send(result) = event else {
                panic!("an unread queue admits the final chunk immediately");
            };
            assert!(result.is_ok(), "the handler is still reading its body");
            streams.complete_send(result);
        })
    });

    settled.expect("the admission settles on a paused runtime");
    assert_eq!(
        counts.count(StreamingBodyOutcome::Delivered),
        1,
        "the accepted final chunk is counted once"
    );
    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        0,
        "a completed body is not abandoned"
    );
}

/// Aborting counts a live exchange and ignores an idle one.
///
/// `abort` runs after a hard budget breach, where the body cannot finish; the
/// exchange it drops is therefore abandoned by definition. A connection with no
/// live body has nothing to report.
#[rstest]
#[test]
fn aborting_counts_only_a_live_exchange(final_chunk: FirstChunk) {
    let ((), counts) = recorded(|| {
        let mut idle = ConnectionStreams::new(QueuedBytes::new());
        idle.abort();
        let mut live = ConnectionStreams::new(QueuedBytes::new());
        live.start(final_chunk, &dropping_handler());
        live.abort();
    });

    assert_eq!(
        counts.count(StreamingBodyOutcome::Abandoned),
        1,
        "only the live exchange's body is abandoned"
    );
}

/// A chunk that never fits the account is counted as an admission timeout.
///
/// The body has not ended and the handler has not answered, so the only thing
/// that happened is that admission kept being refused. Counting it as an
/// abandonment would blame the handler for a chunk it never saw.
#[rstest]
#[test]
fn a_stalled_admission_is_counted_as_an_admission_timeout(final_chunk: FirstChunk) {
    let (settled, timeouts) = counted(StreamingBodyOutcome::AdmissionTimeout, || {
        paused_block_on(async {
            let mut streams = ConnectionStreams::new(QueuedBytes::new());
            let account = streams.queued();
            let (holder, _held) = accounted_body_channel(1, account);
            holder
                .send_chunk(Bytes::from_static(b"held"), 0, usize::MAX)
                .await
                .expect("an empty account admits one chunk");
            streams.start(final_chunk, &reading_route());
            streams
                .drive_pending(
                    wait(),
                    ChunkBudget {
                        other_retained: 0,
                        limit: 6,
                    },
                )
                .await
        })
    });

    assert!(
        matches!(
            settled.expect("the drive runs on a paused runtime"),
            PendingEvent::Idle
        ),
        "a chunk that never fits must hand control back to the loop"
    );
    assert_eq!(timeouts, 1, "the expired admission is counted once");
}

/// Either open body state means the handler lost the rest of its body.
///
/// The `(pending, sender)` shape decides this, and both halves matter: a chunk
/// still waiting for admission is the common case, while an open sender with no
/// chunk queued is the case where a handler resolved before its next
/// continuation arrived. Counting only the pending half would leave that second
/// body uncounted, and counting an already-complete body would report a
/// delivered body as abandoned.
#[rstest]
#[case(true, true, 1)]
#[case(true, false, 1)]
#[case(false, true, 1)]
#[case(false, false, 0)]
#[test]
fn an_exchange_that_ended_before_its_body_is_counted_once(
    #[case] has_pending: bool,
    #[case] has_sender: bool,
    #[case] expected: u64,
) {
    let ((), abandoned) = counted(StreamingBodyOutcome::Abandoned, || {
        let (sender, _body) = accounted_body_channel(1, QueuedBytes::new());
        let pending = has_pending.then(|| PendingSend {
            sender: sender.clone(),
            chunk: Bytes::from_static(b"continuation"),
            is_last: false,
        });
        let open = has_sender.then(|| sender.clone());
        record_unfinished(pending.as_ref(), open.as_ref());
    });

    assert_eq!(
        abandoned, expected,
        "a body is abandoned exactly when one of its open halves survived"
    );
}
