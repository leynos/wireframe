//! Property tests for the byte-accounted request-body channel.
//!
//! The example-based tests name the states that matter. These properties cover
//! the space between them: any ordering of sends, takes and an abandoned
//! consumer, over any chunk sizes and budgets, must leave the account equal to
//! the bytes its live guards still hold, and must refuse exactly the chunks the
//! documented rules refuse.

use std::{collections::VecDeque, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use proptest::prelude::*;
use tokio::time::timeout;

use super::{QueuedBytes, SendChunkError, accounted_body_channel};

/// Chunks a case may queue before its consumer takes any.
///
/// It exceeds the longest generated history, so a send never waits for channel
/// capacity and every observation is about the byte account alone.
const CAPACITY: usize = 16;
/// A zero-length wait, so a refused admission returns after one poll.
const NOW: Duration = Duration::ZERO;
/// Generous bound for an admission the rule allows.
const ADMITS: Duration = Duration::from_secs(5);

/// One operation on the channel.
#[derive(Clone, Debug)]
enum Step {
    /// Offer a chunk of this many bytes.
    Send(usize),
    /// Take one queued value, if the consumer has one.
    Take,
    /// Drop the consumer, closing the channel.
    Abandon,
}

/// Generate a history weighted towards sends, with some takes and one close.
fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (0_usize..=32).prop_map(Step::Send),
        3 => Just(Step::Take),
        1 => Just(Step::Abandon),
    ]
}

/// Report whether `len` bytes may join an account holding `queued` bytes.
///
/// Reimplemented from the documented contract rather than calling the
/// production helper, so the property tests the rule and not a copy of it.
fn admits(queued: usize, other: usize, len: usize, limit: usize) -> bool {
    queued == 0 || queued.saturating_add(other).saturating_add(len) <= limit
}

/// Sum the model's queued chunk lengths.
fn modelled(model: &VecDeque<usize>) -> usize { model.iter().copied().sum() }

proptest! {
    /// The account equals the live guards' bytes after every step.
    ///
    /// A refused chunk leaves nothing charged, a taken chunk releases exactly
    /// its own length, and a dropped consumer releases everything at once. The
    /// aggregate limit binds every chunk admitted to a non-empty account; only
    /// the documented empty-account exception may exceed it, and then only for
    /// the one chunk it let in.
    #[test]
    fn the_account_tracks_exactly_the_queued_bytes(
        steps in prop::collection::vec(step(), 1..10),
        other in 0_usize..=16,
        limit in 0_usize..=64,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| TestCaseError::fail(format!("build a runtime: {error}")))?;
        runtime.block_on(async {
            let queued = QueuedBytes::new();
            let (sender, body) = accounted_body_channel(CAPACITY, queued.clone());
            let mut stream = Some(body);
            let mut model: VecDeque<usize> = VecDeque::new();
            // Keeps the channel open, so only `Abandon` closes it.
            let _keep_alive = sender.clone();

            for step in steps {
                match step {
                    Step::Send(len) => {
                        let fits = admits(modelled(&model), other, len, limit);
                        let settled = timeout(
                            NOW,
                            sender.clone().send_chunk(Bytes::from(vec![0_u8; len]), other, limit),
                        )
                        .await;
                        if fits {
                            prop_assert!(
                                settled.is_ok(),
                                "a chunk the rule admits must not wait: {len} bytes beside {:?}, other {other}, limit {limit}",
                                model
                            );
                            model.push_back(len);
                        } else {
                            prop_assert!(
                                settled.is_err(),
                                "a chunk the rule refuses must wait: {len} bytes beside {:?}, other {other}, limit {limit}",
                                model
                            );
                        }
                        prop_assert_eq!(
                            queued.queued(),
                            modelled(&model),
                            "the account must hold exactly the queued chunk bytes"
                        );
                        // The documented exception admits a chunk to an empty
                        // account, and a zero-length chunk leaves the account
                        // empty, so the exception stays available until a chunk
                        // actually carries bytes. Exceeding the limit therefore
                        // means every chunk before this one was empty.
                        let earlier_are_empty =
                            model.iter().rev().skip(1).all(|queued_len| *queued_len == 0);
                        prop_assert!(
                            queued.queued() <= limit || earlier_are_empty,
                            "only a chunk admitted to an empty account may exceed the limit: {} bytes beside {:?}, limit {limit}",
                            queued.queued(),
                            model
                        );
                    }
                    Step::Take => {
                        let taken = match stream.as_mut() {
                            Some(consumer) => timeout(NOW, consumer.next())
                                .await
                                .ok()
                                .flatten()
                                .and_then(Result::ok),
                            None => None,
                        };
                        match taken {
                            Some(bytes) => {
                                prop_assert_eq!(
                                    Some(bytes.len()),
                                    model.pop_front(),
                                    "the consumer must receive the chunks in order"
                                );
                            }
                            None => prop_assert!(
                                model.is_empty(),
                                "a consumer with nothing queued cannot yield a chunk"
                            ),
                        }
                        prop_assert_eq!(
                            queued.queued(),
                            modelled(&model),
                            "taking a chunk must release exactly its own bytes"
                        );
                    }
                    Step::Abandon => {
                        drop(stream.take());
                        model.clear();
                        prop_assert_eq!(
                            queued.queued(),
                            0,
                            "dropping the consumer must release every queued byte"
                        );
                        let late = sender
                            .clone()
                            .send_chunk(Bytes::from_static(b"late"), other, limit)
                            .await;
                        prop_assert_eq!(
                            late,
                            Err(SendChunkError::Closed),
                            "a closed consumer must refuse the chunk"
                        );
                        prop_assert_eq!(
                            queued.queued(),
                            0,
                            "a refused chunk must charge nothing"
                        );
                        break;
                    }
                }
            }
            Ok(())
        })?;
    }

    /// Admission is decided by the rule, not by the order chunks arrive in.
    ///
    /// Two chunk sizes and one budget are enough to pin the boundary: a chunk
    /// that fits exactly is admitted, and one byte more is not.
    #[test]
    fn admission_follows_the_documented_rule(
        first in 0_usize..=32,
        second in 0_usize..=32,
        other in 0_usize..=16,
        limit in 0_usize..=64,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| TestCaseError::fail(format!("build a runtime: {error}")))?;
        runtime.block_on(async {
            let queued = QueuedBytes::new();
            let (sender, mut body) = accounted_body_channel(CAPACITY, queued.clone());

            let first_admitted = timeout(
                ADMITS,
                sender.clone().send_chunk(Bytes::from(vec![0_u8; first]), other, limit),
            )
            .await
            .is_ok();
            prop_assert!(first_admitted, "an empty account admits one chunk of any size");

            let second_fits = admits(first, other, second, limit);
            let second_settled = timeout(
                NOW,
                sender.clone().send_chunk(Bytes::from(vec![0_u8; second]), other, limit),
            )
            .await;
            prop_assert_eq!(second_settled.is_ok(), second_fits);
            prop_assert_eq!(queued.queued(), if second_fits { first + second } else { first });

            // Taking both releases the account to empty.
            prop_assert!(timeout(ADMITS, body.next()).await.is_ok());
            if second_fits {
                prop_assert!(timeout(ADMITS, body.next()).await.is_ok());
            }
            prop_assert_eq!(queued.queued(), 0, "an empty channel holds no bytes");
            Ok(())
        })?;
    }
}
