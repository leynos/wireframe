//! Unit tests for the parked-message queue bounds.

use std::{collections::VecDeque, future, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use proptest::prelude::*;
use tracing_test::traced_test;

use super::{
    MAX_PARKED_MESSAGES,
    ParkedMessage,
    ParkedQueue,
    PrefilledOffer,
    await_parked_reply,
    deliver_prefilled,
    park,
    parked_bytes,
};
use crate::{
    app::{
        Envelope,
        inbound_handler::{stream_active::HandlerFuture, stream_handles::ChunkBudget},
    },
    request::{QueuedBytes, RequestParts, accounted_body_channel},
};

/// Message identifier the pre-filled delivery tests report under.
const PARKED_MESSAGE_ID: u32 = 10;

/// An empty payload is the case the byte-based memory budget cannot see.
fn empty_envelope() -> ParkedMessage { ParkedMessage::Envelope(Envelope::new(9, None, Vec::new())) }

#[test]
fn park_refuses_to_grow_past_the_cap() {
    let mut parked = ParkedQueue::new();
    for _ in 0..MAX_PARKED_MESSAGES {
        assert!(
            park(&mut parked, empty_envelope()).is_ok(),
            "every message below the cap should be parked"
        );
    }
    assert_eq!(parked.len(), MAX_PARKED_MESSAGES);

    let error = park(&mut parked, empty_envelope())
        .expect_err("a full queue should refuse another message");

    assert_eq!(
        error.kind(),
        std::io::ErrorKind::InvalidData,
        "the refusal should end the connection as invalid peer input"
    );
    assert_eq!(
        parked.len(),
        MAX_PARKED_MESSAGES,
        "a refused message should not be queued"
    );
}

#[test]
fn park_accepts_a_message_once_the_queue_drains() {
    let mut parked = ParkedQueue::new();
    for _ in 0..MAX_PARKED_MESSAGES {
        assert!(park(&mut parked, empty_envelope()).is_ok());
    }
    assert!(
        parked.pop_front().is_some(),
        "the queue should have an entry"
    );

    assert!(
        park(&mut parked, empty_envelope()).is_ok(),
        "draining one message should free a place for the next"
    );
}

/// Bytes an earlier body never released must not block a pre-filled body.
#[tokio::test(start_paused = true)]
async fn an_unaffordable_prefilled_body_becomes_a_terminal_error() {
    let queued = QueuedBytes::new();
    let (holder, _held_body) = accounted_body_channel(1, queued.clone());
    holder
        .send_chunk(Bytes::from_static(b"held"), 0, usize::MAX)
        .await
        .expect("an empty queue admits one chunk");
    let (sender, mut body) = accounted_body_channel(1, queued);

    let offer = PrefilledOffer {
        body: Bytes::from_static(b"abcd"),
        message_id: PARKED_MESSAGE_ID,
    };
    deliver_prefilled(&sender, offer, tight_budget(), wait()).await;

    let item = body
        .next()
        .await
        .expect("the handler must observe the abandoned body");
    assert!(
        item.is_err(),
        "a body that cannot be admitted must fail, not queue silently"
    );
}

#[tokio::test(start_paused = true)]
async fn an_affordable_prefilled_body_arrives_whole() {
    let (sender, mut body) = accounted_body_channel(1, QueuedBytes::new());

    let offer = PrefilledOffer {
        body: Bytes::from_static(b"abcd"),
        message_id: PARKED_MESSAGE_ID,
    };
    deliver_prefilled(&sender, offer, tight_budget(), wait()).await;

    let item = body
        .next()
        .await
        .expect("the assembled body should arrive as one chunk");
    assert_eq!(
        item.expect("the chunk is not an error").as_ref(),
        b"abcd",
        "the whole body should be delivered in one chunk"
    );
}

/// A parked handler that never completes must not hold the connection open.
///
/// The reply is awaited while parked messages drain, which happens during
/// end-of-connection teardown. Without a bound, one unresponsive handler kept
/// the connection from closing and its resources allocated.
#[traced_test]
#[tokio::test(start_paused = true)]
async fn an_unresponsive_prefilled_handler_is_bounded_by_the_wait() {
    let mut response: HandlerFuture = Box::pin(future::pending());

    let payload = await_parked_reply(wait(), &mut response, 7).await;

    assert_eq!(
        payload, None,
        "an elapsed reply wait must be treated as no reply"
    );
    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .any(|line| line.contains("streaming handler exceeded the connection wait"))
            .then_some(())
            .ok_or_else(|| "abandoned handler was not reported".to_owned())
    });
}

#[tokio::test(start_paused = true)]
async fn a_completed_prefilled_handler_reply_is_returned() {
    let mut response: HandlerFuture = Box::pin(async { Some(Bytes::from_static(b"reply")) });

    let payload = await_parked_reply(wait(), &mut response, 7).await;

    assert_eq!(
        payload.as_deref(),
        Some(b"reply".as_slice()),
        "a reply produced within the wait must still be forwarded"
    );
}

/// Allow less room than four body bytes plus the held ones need.
fn tight_budget() -> ChunkBudget {
    ChunkBudget {
        other_retained: 0,
        limit: 6,
    }
}

/// Bound used by the pre-filled delivery tests.
fn wait() -> Duration { Duration::from_millis(50) }

/// The message shape one generated park operation offers.
#[derive(Clone, Copy, Debug)]
enum ParkedShape {
    /// A buffered message with a payload of this many bytes.
    Envelope(usize),
    /// A pre-filled streaming body with this much metadata and body.
    Stream {
        /// Routing metadata bytes the message retains.
        metadata: usize,
        /// Assembled body bytes the message retains.
        body: usize,
    },
}

impl ParkedShape {
    /// Return the bytes this shape contributes to the queue.
    const fn contribution(self) -> usize {
        match self {
            Self::Envelope(payload) => payload,
            Self::Stream { metadata, body } => metadata.saturating_add(body),
        }
    }

    /// Build the message this shape describes.
    fn message(self) -> ParkedMessage {
        match self {
            Self::Envelope(payload) => {
                ParkedMessage::Envelope(Envelope::new(9, None, vec![0; payload]))
            }
            Self::Stream { metadata, body } => ParkedMessage::Stream {
                parts: RequestParts::new(9, None, vec![0; metadata]),
                body: Bytes::from(vec![0; body]),
            },
        }
    }
}

/// One operation on the parked queue.
#[derive(Clone, Copy, Debug)]
enum Operation {
    /// Park a message of the given shape.
    Park(ParkedShape),
    /// Serve the message at the front.
    Drain,
}

/// Generate a history weighted towards parking, so the cap is reached often.
fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        4 => (0_usize..=64).prop_map(|payload| Operation::Park(ParkedShape::Envelope(payload))),
        4 => (0_usize..=16, 0_usize..=64).prop_map(|(metadata, body)| {
            Operation::Park(ParkedShape::Stream { metadata, body })
        }),
        2 => Just(Operation::Drain),
    ]
}

/// Return the bytes one parked message retains, computed independently.
fn retained(message: &ParkedMessage) -> usize {
    match message {
        ParkedMessage::Envelope(envelope) => envelope.payload.len(),
        ParkedMessage::Stream { parts, body } => parts.metadata().len() + body.len(),
    }
}

proptest! {
    /// The queue never exceeds its cap and its bytes match its entries.
    ///
    /// A refused message changes nothing, a served message releases exactly
    /// the bytes it retained, and the total always equals the sum over the
    /// entries still waiting — for buffered payloads and pre-filled bodies
    /// alike.
    #[test]
    fn parking_and_draining_conserve_the_queue(
        operations in prop::collection::vec(operation(), 1..80),
    ) {
        let mut parked = ParkedQueue::new();
        let mut model: VecDeque<usize> = VecDeque::new();
        for step in operations {
            match step {
                Operation::Park(shape) => {
                    let outcome = park(&mut parked, shape.message());
                    if model.len() < MAX_PARKED_MESSAGES {
                        prop_assert!(
                            outcome.is_ok(),
                            "a queue below the cap must accept the message"
                        );
                        model.push_back(shape.contribution());
                    } else {
                        prop_assert!(
                            outcome.is_err(),
                            "a full queue must refuse another message"
                        );
                    }
                }
                Operation::Drain => {
                    let expected = model.pop_front();
                    let drained = parked.pop_front().map(|message| retained(&message));
                    prop_assert_eq!(
                        drained,
                        expected,
                        "draining must release exactly the oldest message's bytes"
                    );
                }
            }
            prop_assert!(
                parked.len() <= MAX_PARKED_MESSAGES,
                "the queue must never exceed its cap"
            );
            prop_assert_eq!(parked.len(), model.len());
            prop_assert_eq!(
                parked_bytes(&parked),
                model.iter().copied().sum::<usize>(),
                "the retained total must equal the sum over the parked entries"
            );
        }
    }
}
