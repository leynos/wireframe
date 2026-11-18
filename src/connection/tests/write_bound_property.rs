//! Property coverage for the bounds that end a pending write.
//!
//! A write is abandoned only when the server stops or the peer has been
//! unobservable for a whole idle grace, and a write that keeps making progress
//! renews that grace instead of losing the rest of its output. These properties
//! generate the timings rather than replaying fixed examples, so a change to
//! the select's precedence or to per-event renewal cannot pass by coincidence.

use std::{
    future::Future,
    io,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use proptest::{prelude::*, test_runner::TestCaseError};
use tokio::{
    runtime::{Builder, Runtime},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use super::test_actor;
use crate::{
    app::Envelope,
    connection::{FrameSink, PushSink, ResponseOutcome, WriteBound, write_before_bound},
    hooks::ProtocolHooks,
    response::Response,
};

/// Build a current-thread runtime whose clock only moves when nothing is
/// ready, which lets the generated deadlines decide the outcome.
fn paused_runtime() -> Result<Runtime, TestCaseError> {
    Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .map_err(|error| TestCaseError::fail(format!("a paused runtime must build: {error}")))
}

/// Sink that accepts each frame only after `pacing` has elapsed.
struct PacedSink {
    /// How long the peer takes to accept one frame.
    pacing: Duration,
    /// Frames the peer accepted, in write order.
    written: Arc<Mutex<Vec<Envelope>>>,
}

impl FrameSink<Envelope> for PacedSink {
    fn send_frame(&mut self, frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        let pacing = self.pacing;
        let written = Arc::clone(&self.written);
        async move {
            tokio::time::sleep(pacing).await;
            // A sink that only ever pushes cannot observe a torn record, so a
            // poisoned lock still holds a usable value.
            written
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(frame);
            Ok(())
        }
    }
}

// These cases queue no pushes, so a push takes the frame path unchanged.
impl PushSink<Envelope> for PacedSink {
    fn send_push(&mut self, frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        self.send_frame(frame)
    }
}

/// Whether the write completes inside `grace`, which is what decides the bound.
const fn completes_first(write_ms: u64, grace_ms: u64) -> bool { write_ms <= grace_ms }

// With no server stop, a pending write is abandoned exactly when the grace
// expires and a write that finishes first is delivered.
proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn a_pending_write_ends_at_whichever_comes_first(
        write_ms in 1u64..=20,
        grace_ms in 1u64..=20,
    ) {
        let runtime = paused_runtime()?;
        let (bound, elapsed) = runtime.block_on(async move {
            let shutdown = CancellationToken::new();
            let started = Instant::now();
            let write = async move {
                tokio::time::sleep(Duration::from_millis(write_ms)).await;
                Ok(())
            };
            let grace = tokio::time::sleep(Duration::from_millis(grace_ms));
            let bound = write_before_bound(write, &shutdown, grace)
                .await
                .map_err(|error| TestCaseError::fail(format!("a bounded write is not an error: {error:?}")))?;
            Ok::<_, TestCaseError>((bound, started.elapsed()))
        })?;

        let expected = if completes_first(write_ms, grace_ms) {
            WriteBound::Finished
        } else {
            WriteBound::GraceExpired
        };
        prop_assert_eq!(bound, expected);
        prop_assert_eq!(elapsed, Duration::from_millis(write_ms.min(grace_ms)));
    }
}

// A server stop outranks the grace, and a write the transport accepts still
// wins over both, so the earliest ready bound is the one reported.
proptest! {
    #[test]
    fn a_stop_outranks_the_grace_but_never_a_finished_write(
        write_ms in 1u64..=20,
        grace_ms in 1u64..=20,
        stop_ms in 1u64..=20,
    ) {
        // A stop delivered by a separate task at the same instant as the grace
        // makes the tie depend on task scheduling rather than on the actor's
        // precedence, so the tie case is not a statement about this code.
        prop_assume!(stop_ms != grace_ms);
        let runtime = paused_runtime()?;
        let bound = runtime.block_on(async move {
            let shutdown = CancellationToken::new();
            let stopper = tokio::spawn({
                let stop_token = shutdown.clone();
                async move {
                    tokio::time::sleep(Duration::from_millis(stop_ms)).await;
                    stop_token.cancel();
                }
            });
            let write = async move {
                tokio::time::sleep(Duration::from_millis(write_ms)).await;
                Ok(())
            };
            let grace = tokio::time::sleep(Duration::from_millis(grace_ms));
            let bound = write_before_bound(write, &shutdown, grace)
                .await
                .map_err(|error| TestCaseError::fail(format!("a bounded write is not an error: {error:?}")))?;
            stopper
                .await
                .map_err(|error| TestCaseError::fail(format!("the stopper must finish: {error}")))?;
            Ok::<_, TestCaseError>(bound)
        })?;

        let expected = if completes_first(write_ms, stop_ms.min(grace_ms)) {
            WriteBound::Finished
        } else if stop_ms <= grace_ms {
            WriteBound::Stopped
        } else {
            WriteBound::GraceExpired
        };
        prop_assert_eq!(bound, expected);
    }
}

// A response whose frames are each accepted within one grace is delivered in
// full even when the whole response takes longer than a grace, because every
// accepted frame renews the deadline. A frame that outlasts the grace abandons
// the response with nothing written.
proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn progress_renews_the_grace(
        frames in 1u32..=4,
        pacing_ms in 1u64..=10,
        grace_ms in 1u64..=12,
    ) {
        let runtime = paused_runtime()?;
        let (outcome, written) = runtime.block_on(async move {
            let mut actor = test_actor(ProtocolHooks::default())
                .map_err(|error| TestCaseError::fail(format!("the actor must build: {error:?}")))?;
            actor.set_peer_closed_grace(Duration::from_millis(grace_ms));
            let peer_closed = CancellationToken::new();
            actor.set_peer_closed(peer_closed.clone());
            let written = Arc::new(Mutex::new(Vec::new()));
            let mut sink = PacedSink {
                pacing: Duration::from_millis(pacing_ms),
                written: Arc::clone(&written),
            };
            let response = Response::Vec(
                (0..frames)
                    .map(|id| Envelope::new(id, None, Vec::new()))
                    .collect(),
            );

            peer_closed.cancel();
            let outcome = actor
                .install_response(response, Some(7), &mut sink)
                .await
                .map_err(|error| TestCaseError::fail(format!("a bounded response is not an error: {error:?}")))?;
            let accepted = written
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len();
            Ok::<_, TestCaseError>((outcome, accepted))
        })?;

        if completes_first(pacing_ms, grace_ms) {
            prop_assert_eq!(outcome, ResponseOutcome::Completed);
            prop_assert_eq!(written, usize::try_from(frames).unwrap_or(usize::MAX));
        } else {
            prop_assert_eq!(outcome, ResponseOutcome::Cancelled);
            prop_assert_eq!(written, 0, "an abandoned first write delivers nothing");
        }
    }
}
