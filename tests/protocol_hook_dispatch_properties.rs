//! Property coverage for the actor's per-frame hook dispatch.
//!
//! The example-based dispatch tests name nine concrete scenarios. This
//! companion checks the invariants those examples share for arbitrary input:
//! whatever frames a connection produces — pushes, a streamed or multi-packet
//! response, and a terminator — `before_send` runs exactly once per emitted
//! frame, in emission order, and the emitted frame is the hooked frame.
//!
//! Generated frames carry a source tag and their position in that source, so a
//! repeated payload cannot hide a duplicate or a missing dispatch. The
//! mixed-source generator and its case runner live in
//! `tests/common/mixed_dispatch_property.rs`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/dispatch_fixtures.rs"]
mod dispatch_fixtures;
#[path = "common/mixed_dispatch_property.rs"]
mod mixed_dispatch_property;
#[path = "common/recording_protocol.rs"]
mod recording_protocol;

use std::sync::Arc;

use dispatch_fixtures::{actor_with_hooks, queues, run_error};
use mixed_dispatch_property::{
    push_strategy,
    response_strategy,
    run_mixed_property_case,
    terminator_strategy,
};
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use recording_protocol::{HookEvent, MARKER, RecordingProtocolFixture};
use tokio::runtime::Builder;
use wireframe::hooks::ProtocolHooks;
use wireframe_testing::TestResult;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn every_pushed_frame_is_hooked_once_in_order(
        frames in prop::collection::vec(
            prop::collection::vec(any::<u8>(), 1..=16),
            1..=6,
        ),
    ) {
        run_push_property_case(&frames)?;
    }

    /// Pushes, one response source, and a terminator share one hook owner.
    ///
    /// A `ConnectionActor` keeps one response source active at a time, so each
    /// case generates pushes plus either a streamed response or a multi-packet
    /// channel. Both variants are exercised across the sampled cases.
    #[test]
    fn every_emitted_frame_is_hooked_once_in_emission_order(
        pushes in push_strategy(),
        response in response_strategy(),
        terminator in terminator_strategy(),
    ) {
        run_mixed_property_case(&pushes, response, terminator)?;
    }
}

/// Drive one generated push sequence on a deterministic Tokio runtime.
fn run_push_property_case(frames: &[Vec<u8>]) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime
        .block_on(exercise_push_property_case(frames))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Push every generated frame and compare dispatch order with emission order.
async fn exercise_push_property_case(frames: &[Vec<u8>]) -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let (queues, handle) = queues()?;
    for frame in frames {
        handle.push_high_priority(frame.clone()).await?;
    }
    let mut actor = actor_with_hooks(
        queues,
        handle,
        None,
        ProtocolHooks::from_protocol(&Arc::new(fixture.protocol())),
    );

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|error| run_error(&error))?;

    let events = fixture.events();
    assert_eq!(
        events.first(),
        Some(&HookEvent::Setup),
        "setup runs once before the first frame"
    );
    let observed: Vec<&Vec<u8>> = events
        .iter()
        .filter_map(|event| match event {
            HookEvent::BeforeSend(frame) => Some(frame),
            _ => None,
        })
        .collect();
    let sent: Vec<&Vec<u8>> = frames.iter().collect();
    assert_eq!(
        observed, sent,
        "before_send observes each pushed frame exactly once, in order"
    );

    let expected: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| {
            let mut marked = frame.clone();
            marked.push(MARKER);
            marked
        })
        .collect();
    assert_eq!(
        out, expected,
        "the peer receives each frame with exactly one hook mutation"
    );
    Ok(())
}
