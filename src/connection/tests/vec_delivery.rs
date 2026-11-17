//! Property tests for multi-frame (`Response::Vec`) delivery.
//!
//! A handler may answer with any sequence of frames. Delivery has to preserve
//! that order, apply the request's correlation identifier to every frame when
//! the request had one (and otherwise leave each frame's own identifier
//! alone), stop at the first sink failure, and signal command completion
//! exactly once and only after the whole response is written.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use proptest::prelude::*;

use super::{counting_hooks, test_actor};
use crate::{
    app::Envelope,
    connection::{FrameSink, ResponseOutcome},
    response::{Response, WireframeError},
};

/// One frame a handler supplies: identity, payload, and its own correlation.
type FrameSpec = (u8, u8, Option<u64>);

/// Sink that records written frames and refuses one write.
struct FailingSink {
    /// Frames accepted before the refused write.
    written: Vec<Envelope>,
    /// Position at which a write is refused, if any.
    fail_at: Option<usize>,
}

impl FailingSink {
    /// Build a sink that accepts every write.
    const fn writable() -> Self {
        Self {
            written: Vec::new(),
            fail_at: None,
        }
    }

    /// Build a sink that refuses the write at `position`.
    const fn failing_at(position: usize) -> Self {
        Self {
            written: Vec::new(),
            fail_at: Some(position),
        }
    }
}

impl FrameSink<Envelope> for FailingSink {
    fn send_frame(&mut self, frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        if self.fail_at == Some(self.written.len()) {
            return std::future::ready(Err(io::Error::other("sink refused the write")));
        }
        self.written.push(frame);
        std::future::ready(Ok(()))
    }
}

/// Strategy for one supplied frame.
fn frame_spec() -> impl Strategy<Value = FrameSpec> {
    (any::<u8>(), any::<u8>(), prop::option::of(any::<u64>()))
}

/// Request correlation applied to a delivered frame.
///
/// The actor stamps the request's identifier over every frame when the request
/// supplied one, and otherwise leaves the handler's own identifier in place.
const fn expected_correlation(
    request_correlation: Option<u64>,
    own_correlation: Option<u64>,
) -> Option<u64> {
    match request_correlation {
        Some(correlation) => Some(correlation),
        None => own_correlation,
    }
}

/// The frames a handler would return, in the order they were supplied.
fn frames_from(specs: &[FrameSpec]) -> Vec<Envelope> {
    specs
        .iter()
        .map(|&(id, payload, correlation)| Envelope::new(u32::from(id), correlation, vec![payload]))
        .collect()
}

/// The identity, payload, and correlation each delivered frame must carry.
fn expected_frames(
    specs: &[FrameSpec],
    request_correlation: Option<u64>,
) -> Vec<(u32, Vec<u8>, Option<u64>)> {
    specs
        .iter()
        .map(|&(id, payload, own_correlation)| {
            (
                u32::from(id),
                vec![payload],
                expected_correlation(request_correlation, own_correlation),
            )
        })
        .collect()
}

/// Read the identity, payload, and correlation off a delivered frame.
fn observed(frame: &Envelope) -> (u32, Vec<u8>, Option<u64>) {
    (frame.id, frame.payload.clone(), frame.correlation_id)
}

/// Build a single-threaded runtime for one property case.
fn test_runtime() -> Result<tokio::runtime::Runtime, TestCaseError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(format!("the runtime should build: {error}")))
}

proptest! {
    /// Frames are delivered in order with the request's correlation applied.
    #[test]
    fn delivery_preserves_order_and_inherits_correlation(
        specs in prop::collection::vec(frame_spec(), 0..8),
        request_correlation in prop::option::of(any::<u64>()),
    ) {
        let runtime = test_runtime()?;
        let (outcome, delivered, calls) = runtime.block_on(async {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut actor = test_actor(counting_hooks(Arc::clone(&calls)))
                .expect("the actor should build");
            let mut sink = FailingSink::writable();

            let outcome = actor
                .install_response(
                    Response::Vec(frames_from(&specs)),
                    request_correlation,
                    &mut sink,
                )
                .await
                .expect("a writable sink completes the response");
            let delivered: Vec<_> = sink.written.iter().map(observed).collect();
            (outcome, delivered, calls.load(Ordering::SeqCst))
        });

        prop_assert_eq!(outcome, ResponseOutcome::Completed);
        prop_assert_eq!(
            delivered,
            expected_frames(&specs, request_correlation),
            "every frame must arrive in order with the expected correlation",
        );
        prop_assert_eq!(
            calls,
            1,
            "a completed response signals command completion exactly once",
        );
    }

    /// A refused write stops delivery after the frames already written.
    #[test]
    fn delivery_stops_at_the_first_sink_failure(
        (specs, fail_at) in prop::collection::vec(frame_spec(), 1..8).prop_flat_map(|specs| {
            let positions = 0..specs.len();
            (Just(specs), positions)
        }),
        request_correlation in prop::option::of(any::<u64>()),
    ) {
        let runtime = test_runtime()?;
        let (is_io_error, delivered, calls) = runtime.block_on(async {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut actor = test_actor(counting_hooks(Arc::clone(&calls)))
                .expect("the actor should build");
            let mut sink = FailingSink::failing_at(fail_at);

            let result = actor
                .install_response(
                    Response::Vec(frames_from(&specs)),
                    request_correlation,
                    &mut sink,
                )
                .await;
            let is_io_error = matches!(result, Err(WireframeError::Io(_)));
            let delivered: Vec<_> = sink.written.iter().map(observed).collect();
            (is_io_error, delivered, calls.load(Ordering::SeqCst))
        });

        prop_assert!(is_io_error, "a refused write surfaces as an I/O error");
        prop_assert_eq!(
            delivered,
            expected_frames(&specs, request_correlation)
                .into_iter()
                .take(fail_at)
                .collect::<Vec<_>>(),
            "exactly the frames before the refused write are delivered",
        );
        prop_assert_eq!(
            calls,
            0,
            "a failed response must not signal command completion",
        );
    }
}
