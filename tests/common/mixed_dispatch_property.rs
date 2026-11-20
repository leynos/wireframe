//! Generated mixed-source cases for the hook-dispatch property test.
//!
//! One case combines pushes of both priorities with one response source and an
//! optional terminator. A `ConnectionActor` keeps one response source active at
//! a time, so a case cannot carry both response shapes; the generator selects
//! one per case, and the property samples both. Generated frames carry a source
//! tag and their position in that source, so a repeated payload cannot hide a
//! duplicate or a missing dispatch. Include this module from
//! `protocol_hook_dispatch_properties` after `dispatch_fixtures` and
//! `recording_protocol`:
//! `#[path = "common/mixed_dispatch_property.rs"] mod mixed_dispatch_property;`.

use std::{slice::Iter, sync::Arc};

use futures::stream;
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use tokio::{runtime::Builder, sync::mpsc};
use wireframe::{
    hooks::ProtocolHooks,
    push::{PushHandle, PushQueues},
    response::{FrameStream, WireframeError},
};
use wireframe_testing::TestResult;

use super::{
    dispatch_fixtures::{actor_with_hooks, queues, run_error},
    recording_protocol::{HookEvent, MARKER, RecordingProtocolFixture},
};

/// Source tag for a high-priority push.
const HIGH_TAG: u8 = 0x01;
/// Source tag for a low-priority push.
const LOW_TAG: u8 = 0x02;
/// Source tag for a streamed response frame.
const STREAM_TAG: u8 = 0x03;
/// Source tag for a multi-packet channel frame.
const MULTI_TAG: u8 = 0x04;
/// Source tag for the stream-end terminator.
const TERMINATOR_TAG: u8 = 0x05;

/// Iterator over the recorded events, consumed by the grammar checks.
type EventIter<'a> = Iter<'a, HookEvent<Vec<u8>>>;

/// A push generated for one case, in generation order.
#[derive(Clone, Debug)]
pub struct GeneratedPush {
    /// Whether the frame is queued on the urgent lane.
    is_high_priority: bool,
    /// Tagged payload, unique within its source.
    payload: Vec<u8>,
}

impl GeneratedPush {
    /// Queue this push on the lane its priority selects.
    async fn queue(&self, handle: &PushHandle<Vec<u8>>) -> TestResult<()> {
        if self.is_high_priority {
            handle.push_high_priority(self.payload.clone()).await?;
        } else {
            handle.push_low_priority(self.payload.clone()).await?;
        }
        Ok(())
    }
}

/// The response source a generated case installs on the actor.
#[derive(Clone, Debug)]
pub enum ResponseSource {
    /// A finite streamed response.
    Streamed(Vec<Vec<u8>>),
    /// A multi-packet channel closed after its frames are queued.
    MultiPacket(Vec<Vec<u8>>),
}

impl ResponseSource {
    /// Frames this source emits, in emission order.
    fn frames(&self) -> &[Vec<u8>] {
        match self {
            Self::Streamed(frames) | Self::MultiPacket(frames) => frames,
        }
    }

    /// Tag shared by this source's generated frames.
    const fn tag(&self) -> u8 {
        match self {
            Self::Streamed(_) => STREAM_TAG,
            Self::MultiPacket(_) => MULTI_TAG,
        }
    }

    /// Split this source into the actor inputs that carry it.
    async fn into_parts(
        self,
    ) -> (
        Option<FrameStream<Vec<u8>, ()>>,
        Option<mpsc::Receiver<Vec<u8>>>,
    ) {
        match self {
            Self::Streamed(frames) => (Some(boxed_stream(frames)), None),
            Self::MultiPacket(frames) => (None, Some(queued_channel(frames).await)),
        }
    }
}

/// Prefix `payload` with a source tag and its position in that source.
///
/// Generated payloads may repeat, so the prefix is what makes each frame
/// identifiable in the recorded event log.
fn tagged(tag: u8, index: usize, payload: Vec<u8>) -> Vec<u8> {
    let position = u8::try_from(index).unwrap_or(u8::MAX);
    let mut frame = vec![tag, position];
    frame.extend(payload);
    frame
}

/// Tag every frame of one source with its position.
fn tagged_frames(tag: u8, frames: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    frames
        .into_iter()
        .enumerate()
        .map(|(index, payload)| tagged(tag, index, payload))
        .collect()
}

/// Generate pushes of both priorities in a fixed generation order.
pub fn push_strategy() -> impl Strategy<Value = Vec<GeneratedPush>> {
    prop::collection::vec(
        (any::<bool>(), prop::collection::vec(any::<u8>(), 0..=4)),
        0..=5,
    )
    .prop_map(|items| {
        items
            .into_iter()
            .enumerate()
            .map(|(index, (is_high_priority, payload))| {
                let tag = if is_high_priority { HIGH_TAG } else { LOW_TAG };
                GeneratedPush {
                    is_high_priority,
                    payload: tagged(tag, index, payload),
                }
            })
            .collect()
    })
}

/// Generate one of the two response source shapes with 0 to 3 frames.
pub fn response_strategy() -> impl Strategy<Value = ResponseSource> {
    let streamed = prop::collection::vec(prop::collection::vec(any::<u8>(), 0..=4), 0..=3)
        .prop_map(|frames| ResponseSource::Streamed(tagged_frames(STREAM_TAG, frames)));
    let multi_packet = prop::collection::vec(prop::collection::vec(any::<u8>(), 0..=4), 0..=3)
        .prop_map(|frames| ResponseSource::MultiPacket(tagged_frames(MULTI_TAG, frames)));
    prop_oneof![streamed, multi_packet]
}

/// Generate an optional stream-end terminator.
pub fn terminator_strategy() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop::option::of(prop::collection::vec(any::<u8>(), 0..=4))
        .prop_map(|payload| payload.map(|bytes| tagged(TERMINATOR_TAG, 0, bytes)))
}

/// Drive one generated mixed-source case on a deterministic Tokio runtime.
pub fn run_mixed_property_case(
    pushes: &[GeneratedPush],
    response: ResponseSource,
    terminator: Option<Vec<u8>>,
) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime
        .block_on(exercise_mixed_property_case(pushes, response, terminator))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Emit every generated frame and check the hook log against the sources.
async fn exercise_mixed_property_case(
    pushes: &[GeneratedPush],
    response: ResponseSource,
    terminator: Option<Vec<u8>>,
) -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(terminator.clone());
    let (queues, handle) = queues()?;
    for push in pushes {
        push.queue(&handle).await?;
    }
    let response_tag = response.tag();
    let expected_response = response.frames().to_vec();
    let out = drive_actor(queues, handle, response, &fixture).await?;

    let events = fixture.events();
    let hooked =
        before_send_order(&events, terminator.as_deref()).map_err(std::io::Error::other)?;
    assert_hooked_output(&out, &hooked);

    let produced = non_terminator_frames(hooked);
    assert_source_order(&produced, pushes, response_tag, &expected_response);
    assert_every_frame_once(produced, pushes, &expected_response);
    Ok(())
}

/// Run the actor over the generated sources and return what the peer received.
async fn drive_actor(
    queues: PushQueues<Vec<u8>>,
    handle: PushHandle<Vec<u8>>,
    response: ResponseSource,
    fixture: &RecordingProtocolFixture<Vec<u8>>,
) -> TestResult<Vec<Vec<u8>>> {
    let (response_stream, multi_packet) = response.into_parts().await;
    let mut actor = actor_with_hooks(
        queues,
        handle,
        response_stream,
        ProtocolHooks::from_protocol(&Arc::new(fixture.protocol())),
    );
    if let Some(rx) = multi_packet {
        actor
            .set_multi_packet(Some(rx))
            .map_err(|error| std::io::Error::other(format!("set multi-packet: {error}")))?;
    }
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|error| run_error(&error))?;
    Ok(out)
}

/// Assert the peer received exactly the hooked frames, each mutated once.
fn assert_hooked_output(out: &[Vec<u8>], hooked: &[Vec<u8>]) {
    let expected: Vec<Vec<u8>> = hooked.iter().map(|frame| with_marker(frame)).collect();
    assert_eq!(
        out, expected,
        "the peer receives every hooked frame once, in hook order"
    );
}

/// Append the recording protocol's mutation to one frame.
fn with_marker(frame: &[u8]) -> Vec<u8> {
    let mut marked = frame.to_vec();
    marked.push(MARKER);
    marked
}

/// Drop the terminator frames, which belong to no generated source.
fn non_terminator_frames(frames: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    frames
        .into_iter()
        .filter(|frame| frame.first() != Some(&TERMINATOR_TAG))
        .collect()
}

/// Assert each source's frames keep the order it queued them in.
fn assert_source_order(
    produced: &[Vec<u8>],
    pushes: &[GeneratedPush],
    response_tag: u8,
    expected_response: &[Vec<u8>],
) {
    let observed = |tag: u8| -> Vec<Vec<u8>> {
        produced
            .iter()
            .filter(|frame| frame.first() == Some(&tag))
            .cloned()
            .collect()
    };
    let expected_pushes = |is_high: bool| -> Vec<Vec<u8>> {
        pushes
            .iter()
            .filter(|push| push.is_high_priority == is_high)
            .map(|push| push.payload.clone())
            .collect()
    };
    assert_eq!(
        observed(HIGH_TAG),
        expected_pushes(true),
        "high-priority pushes keep their queue order"
    );
    assert_eq!(
        observed(LOW_TAG),
        expected_pushes(false),
        "low-priority pushes keep their queue order"
    );
    assert_eq!(
        observed(response_tag),
        expected_response,
        "response frames keep their emission order"
    );
}

/// Assert every generated frame is hooked exactly once.
fn assert_every_frame_once(
    produced: Vec<Vec<u8>>,
    pushes: &[GeneratedPush],
    expected_response: &[Vec<u8>],
) {
    let mut observed = produced;
    observed.sort();
    let mut expected: Vec<Vec<u8>> = pushes.iter().map(|push| push.payload.clone()).collect();
    expected.extend_from_slice(expected_response);
    expected.sort();
    assert_eq!(observed, expected, "every generated frame is hooked once");
}

/// Validate the event grammar and return every frame `before_send` observed.
///
/// The actor dispatches `StreamEnd`, then the terminator when the protocol
/// supplies one, then the deferred completion hook, so one source completion
/// is the contiguous tail `StreamEnd, [BeforeSend(terminator)], CommandEnd`.
/// Any other event is a dispatch the contract does not allow.
fn before_send_order(
    events: &[HookEvent<Vec<u8>>],
    terminator: Option<&[u8]>,
) -> Result<Vec<Vec<u8>>, String> {
    let mut iter = events.iter();
    if iter.next() != Some(&HookEvent::Setup) {
        return Err(format!("setup must run first, got {events:?}"));
    }
    let mut frames = Vec::new();
    let mut completions = 0_usize;
    while let Some(event) = iter.next() {
        match event {
            HookEvent::BeforeSend(frame) => frames.push(frame.clone()),
            HookEvent::StreamEnd => {
                completions += 1;
                collect_completion(&mut iter, terminator, &mut frames)?;
            }
            other => return Err(format!("unexpected event {other:?}")),
        }
    }
    if completions != 1 {
        return Err(format!(
            "expected exactly one source completion, got {completions}: {events:?}"
        ));
    }
    Ok(frames)
}

/// Consume the terminator and completion hooks that follow a stream end.
fn collect_completion(
    rest: &mut EventIter<'_>,
    terminator: Option<&[u8]>,
    frames: &mut Vec<Vec<u8>>,
) -> Result<(), String> {
    if let Some(expected) = terminator {
        take_terminator(rest, expected, frames)?;
    }
    take_command_end(rest)
}

/// Consume the protocol-supplied terminator that follows a stream end.
fn take_terminator(
    rest: &mut EventIter<'_>,
    expected: &[u8],
    frames: &mut Vec<Vec<u8>>,
) -> Result<(), String> {
    match rest.next() {
        Some(HookEvent::BeforeSend(frame)) if frame.as_slice() == expected => {
            frames.push(frame.clone());
            Ok(())
        }
        other => Err(format!(
            "expected the terminator frame after stream end, got {other:?}"
        )),
    }
}

/// Consume the deferred completion hook that ends a source.
fn take_command_end(rest: &mut EventIter<'_>) -> Result<(), String> {
    match rest.next() {
        Some(HookEvent::CommandEnd) => Ok(()),
        other => Err(format!("expected command end, got {other:?}")),
    }
}

/// Box a finite streamed response over `frames`.
fn boxed_stream(frames: Vec<Vec<u8>>) -> FrameStream<Vec<u8>, ()> {
    let items: Vec<Result<Vec<u8>, WireframeError<()>>> = frames.into_iter().map(Ok).collect();
    Box::pin(stream::iter(items))
}

/// Queue every frame and close the sender so the actor sees a complete source.
async fn queued_channel(frames: Vec<Vec<u8>>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel(frames.len().max(1));
    for frame in frames {
        // The receiver is installed on the actor, so the channel cannot be
        // closed while these sends run.
        let _sent = tx.send(frame).await;
    }
    drop(tx);
    rx
}
