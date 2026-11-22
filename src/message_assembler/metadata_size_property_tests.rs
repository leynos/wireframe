//! Property tests for the metadata-inclusive per-message limit (issue #825).
//!
//! The example-based cases in `metadata_size_tests.rs` name the reported
//! protocol outcomes. These properties cover the open-ended space between
//! them, comparing the production state machine against an independent model
//! that counts retained metadata plus body for the limit and body bytes only
//! for the declaration.

use std::{num::NonZeroUsize, time::Duration};

use proptest::prelude::*;

use crate::message_assembler::{
    AssembledMessage,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    test_helpers::continuation_header,
};

// =============================================================================
// Length generators
// =============================================================================

/// A non-zero limit derived from a generated length.
fn non_zero(value: usize) -> NonZeroUsize { NonZeroUsize::new(value).unwrap_or(NonZeroUsize::MIN) }

// =============================================================================
// Model-based property
// =============================================================================

/// One generated message layout and the limit it is measured against.
#[derive(Clone, Debug)]
struct Request {
    /// Metadata bytes carried by the first frame.
    metadata: Vec<u8>,
    /// Body bytes carried by the first frame.
    first_body: Vec<u8>,
    /// Body bytes carried by each continuation, in order.
    continuations: Vec<Vec<u8>>,
    /// Declared total body length, when the protocol supplies one.
    declared: Option<usize>,
    /// The per-message limit.
    cap: usize,
}

impl Request {
    /// Whether the first frame also completes the message.
    fn first_is_last(&self) -> bool { self.continuations.is_empty() }
}

/// What the contract requires of one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    /// The frame was retained and the message is still incomplete.
    Buffered,
    /// The frame completed the message and dispatched these bytes.
    Dispatched { metadata: Vec<u8>, body: Vec<u8> },
    /// The limit rejected the frame with this attempted length.
    TooLarge { attempted: usize },
    /// The first frame's own body contradicted its declaration.
    DeclaredTooSmall { declared: usize, body_len: usize },
    /// A continuation would carry the body past the declaration.
    DeclaredExceeded { declared: usize, attempted: usize },
    /// A completed body disagreed with the declaration.
    DeclaredMismatch { declared: usize, actual: usize },
}

/// Choose a limit below, at, or above `retained`.
///
/// `at_or_above` selects the branch. Sitting strictly below the full retained
/// size is what makes a *continuation* rejection reachable: a cap at or above
/// it admits every prefix, so only first-frame overruns could ever be
/// generated. Both branches clamp to one byte so the result is always a valid
/// `NonZeroUsize`, and the subtraction saturates so a small message cannot
/// underflow.
fn choose_cap(retained: usize, at_or_above: bool, offset: usize) -> usize {
    if at_or_above {
        retained.saturating_add(offset)
    } else {
        retained.saturating_sub(offset)
    }
    .max(1)
}

/// The generator must be able to place the limit below what the message
/// retains, otherwise no continuation could ever be rejected and the property
/// would only ever exercise first-frame overruns.
#[test]
fn choose_cap_straddles_the_retained_size() {
    assert_eq!(
        choose_cap(10, false, 0),
        10,
        "an offset of zero sits exactly at the retained size"
    );
    assert_eq!(
        choose_cap(10, false, 6),
        4,
        "the below branch tightens the limit under the retained size"
    );
    assert_eq!(
        choose_cap(10, true, 6),
        16,
        "the at-or-above branch loosens the limit"
    );
    assert_eq!(
        choose_cap(3, false, 6),
        1,
        "the below branch saturates to a valid limit rather than zero"
    );
    assert_eq!(
        choose_cap(0, false, 0),
        1,
        "an empty message still yields a valid limit"
    );
}

/// Generate a layout whose limit sits below, at, or above the full retained
/// size, so both the accepting and the rejecting partitions are exercised.
fn arbitrary_request() -> impl Strategy<Value = Request> {
    (
        proptest::collection::vec(any::<u8>(), 0..=4),
        proptest::collection::vec(any::<u8>(), 0..=4),
        proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..=4), 0..=3),
        proptest::option::of(0usize..=8),
        any::<bool>(),
        0usize..=6,
    )
        .prop_map(
            |(metadata, first_body, continuations, declared, at_or_above, offset)| {
                let retained = metadata.len()
                    + first_body.len()
                    + continuations.iter().map(Vec::len).sum::<usize>();
                Request {
                    metadata,
                    first_body,
                    continuations,
                    declared,
                    cap: choose_cap(retained, at_or_above, offset),
                }
            },
        )
}

/// The step the first frame's own declaration provokes, if any.
fn declaration_rejection(request: &Request) -> Option<Step> {
    let declared = request.declared?;
    let body_len = request.first_body.len();
    if declared < body_len {
        return Some(Step::DeclaredTooSmall { declared, body_len });
    }
    if request.first_is_last() && declared != body_len {
        return Some(Step::DeclaredMismatch {
            declared,
            actual: body_len,
        });
    }
    None
}

/// The step the limit provokes on the first frame, if it is over the cap.
///
/// The declaration may only tighten the measurement, so the larger of the
/// actual body and the declared total is counted.
fn first_frame_rejection(request: &Request) -> Option<Step> {
    let counted_body = request.declared.unwrap_or(0).max(request.first_body.len());
    match request.metadata.len().checked_add(counted_body) {
        None => Some(Step::TooLarge {
            attempted: usize::MAX,
        }),
        Some(attempted) if attempted > request.cap => Some(Step::TooLarge { attempted }),
        Some(_) => None,
    }
}

/// The step a continuation provokes, if the limit or the declaration refuses
/// it.
///
/// The limit is checked first and counts retained metadata plus body; the
/// declaration counts body bytes only.
fn continuation_rejection(request: &Request, body: &[u8], chunk_len: usize) -> Option<Step> {
    let measured = request
        .metadata
        .len()
        .checked_add(body.len())
        .and_then(|retained| retained.checked_add(chunk_len));
    let Some(attempted) = measured else {
        return Some(Step::TooLarge {
            attempted: usize::MAX,
        });
    };
    if attempted > request.cap {
        return Some(Step::TooLarge { attempted });
    }

    let declared = request.declared?;
    match body.len().checked_add(chunk_len) {
        None => Some(Step::DeclaredExceeded {
            declared,
            attempted: usize::MAX,
        }),
        Some(total) if total > declared => Some(Step::DeclaredExceeded {
            declared,
            attempted: total,
        }),
        Some(_) => None,
    }
}

/// The step a completing body provokes when it disagrees with the declaration.
fn completion_rejection(request: &Request, actual: usize) -> Option<Step> {
    let declared = request.declared?;
    (actual != declared).then_some(Step::DeclaredMismatch { declared, actual })
}

/// The step continuation `chunk` produces, appending it to `body` when the
/// frame is admitted.
fn advance_continuation(
    request: &Request,
    body: &mut Vec<u8>,
    chunk: &[u8],
    is_last: bool,
) -> Step {
    if let Some(rejection) = continuation_rejection(request, body, chunk.len()) {
        return rejection;
    }
    body.extend_from_slice(chunk);
    if !is_last {
        return Step::Buffered;
    }
    completion_rejection(request, body.len()).unwrap_or_else(|| Step::Dispatched {
        metadata: request.metadata.clone(),
        body: body.clone(),
    })
}

/// The steps for the continuation frames, stopping after the one that ends the
/// run.
fn continuation_steps(request: &Request) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut body = request.first_body.clone();
    for (index, chunk) in request.continuations.iter().enumerate() {
        let is_last = index + 1 == request.continuations.len();
        let step = advance_continuation(request, &mut body, chunk, is_last);
        let ends_run = !matches!(step, Step::Buffered);
        steps.push(step);
        if ends_run {
            break;
        }
    }
    steps
}

/// Reimplement the admission contract independently of the production helpers.
///
/// Stages run in the documented order and the first rejection ends the
/// message, so the returned vector stops there.
fn expected_steps(request: &Request) -> Vec<Step> {
    // A declaration the first frame contradicts is checked before the limit.
    let rejected = declaration_rejection(request).or_else(|| first_frame_rejection(request));
    if let Some(step) = rejected {
        return vec![step];
    }
    if request.first_is_last() {
        return vec![Step::Dispatched {
            metadata: request.metadata.clone(),
            body: request.first_body.clone(),
        }];
    }
    let mut steps = vec![Step::Buffered];
    steps.extend(continuation_steps(request));
    steps
}

/// Translate one state-machine result into the model's vocabulary.
fn observed_step(result: Result<Option<AssembledMessage>, MessageAssemblyError>) -> Step {
    match result {
        Ok(Some(message)) => Step::Dispatched {
            metadata: message.metadata().to_vec(),
            body: message.body().to_vec(),
        },
        Ok(None) => Step::Buffered,
        Err(MessageAssemblyError::MessageTooLarge { attempted, .. }) => {
            Step::TooLarge { attempted }
        }
        Err(MessageAssemblyError::DeclaredTotalTooSmall {
            declared, body_len, ..
        }) => Step::DeclaredTooSmall { declared, body_len },
        Err(MessageAssemblyError::DeclaredTotalExceeded {
            declared,
            attempted,
            ..
        }) => Step::DeclaredExceeded {
            declared,
            attempted,
        },
        Err(MessageAssemblyError::DeclaredTotalMismatch {
            declared, actual, ..
        }) => Step::DeclaredMismatch { declared, actual },
        Err(other) => panic!("unexpected assembly error: {other:?}"),
    }
}

/// Feed the first frame described by `request`.
fn feed_first(state: &mut MessageAssemblyState, request: &Request) -> Step {
    let header = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: request.metadata.len(),
        body_len: request.first_body.len(),
        total_body_len: request.declared,
        is_last: request.first_is_last(),
    };
    let input = FirstFrameInput::new(
        &header,
        EnvelopeRouting::default(),
        request.metadata.clone(),
        &request.first_body,
    )
    .expect("header lengths match the payloads");
    observed_step(state.accept_first_frame(input))
}

/// Feed continuation `index` of `request`.
fn feed_continuation(state: &mut MessageAssemblyState, request: &Request, index: usize) -> Step {
    let Some(chunk) = request.continuations.get(index) else {
        panic!("the model only ever names continuations that exist");
    };
    let header = continuation_header(
        1,
        u32::try_from(index + 1).expect("fewer than u32::MAX frames"),
        chunk.len(),
        index + 1 == request.continuations.len(),
    );
    observed_step(state.accept_continuation_frame(&header, chunk))
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// The state machine matches an independent model for an arbitrary
    /// partition of a message across a first frame and up to three
    /// continuations, including partitions that fall short or overshoot both
    /// the limit and the declaration.
    #[test]
    fn partitioned_frames_match_the_metadata_inclusive_limit_model(
        request in arbitrary_request(),
    ) {
        let expected = expected_steps(&request);
        let mut state =
            MessageAssemblyState::new(non_zero(request.cap), Duration::from_secs(30));
        let frames: Vec<&[u8]> = std::iter::once(request.first_body.as_slice())
            .chain(request.continuations.iter().map(Vec::as_slice))
            .collect();
        let mut admitted = 0usize;

        for (index, (expected_step, frame)) in expected.iter().zip(frames.iter()).enumerate() {
            let step = if index == 0 {
                feed_first(&mut state, &request)
            } else {
                feed_continuation(&mut state, &request, index - 1)
            };
            prop_assert_eq!(&step, expected_step);

            // Accounting is observable after every frame, not only once the
            // message ends: a buffered assembly reports its metadata plus each
            // body byte admitted so far.
            if matches!(expected_step, Step::Buffered) {
                admitted += frame.len();
                prop_assert_eq!(
                    state.buffered_count(),
                    1,
                    "a buffered step leaves exactly one assembly"
                );
                prop_assert_eq!(
                    state.total_buffered_bytes(),
                    request.metadata.len() + admitted,
                    "retained bytes count metadata plus every admitted body byte"
                );
            }
        }

        // The generator always makes the final frame final, so the run ends on
        // a terminal step and nothing may remain retained.
        prop_assert_eq!(state.buffered_count(), 0, "a terminal step frees the assembly");
        prop_assert_eq!(
            state.total_buffered_bytes(),
            0,
            "a terminal step retains nothing"
        );
    }
}
