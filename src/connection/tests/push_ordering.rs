//! Property coverage for push-lane ordering and close-time draining.
//!
//! The actor polls urgent frames before best-effort ones, yields to the low
//! lane only after a configured burst, and takes both lanes in priority order
//! when the peer stops writing. These properties generate interleavings of the
//! two lanes rather than replaying a fixed example, so a change to the biased
//! select, the fairness threshold, or the drain order cannot satisfy them by
//! accident.

use std::time::Duration;

use proptest::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::{
    app::Envelope,
    connection::{ConnectionActor, ConnectionChannels, UntrackedActorInit},
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushError, PushHandle, PushPolicy, PushPriority, PushQueues},
};

/// Consecutive urgent frames the default policy delivers before it yields.
const FAIRNESS_THRESHOLD: usize = 8;

/// How long a generated case may wait for the actor to produce its next batch.
///
/// A regression that drops a queued frame leaves the lanes open and empty, so
/// an unbounded wait would hang the test binary instead of reporting the case.
const COLLECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// One queued frame: which lane it went to, and the byte that identifies it.
type LaneFrame = (bool, u8);

/// Build an actor whose lanes accept every generated interleaving.
fn actor_with_lanes() -> Result<(ConnectionActor<Envelope, ()>, PushHandle<Envelope>), TestCaseError>
{
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(64)
        .low_capacity(64)
        .build()
        .map_err(|error| TestCaseError::fail(format!("the queues must build: {error}")))?;
    let actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle.clone()),
        response: None,
        shutdown: CancellationToken::new(),
        hooks: ProtocolHooks::default(),
        ctx: ConnectionContext::default(),
    });
    Ok((actor, handle))
}

/// Queue every frame, in order, through the lane its marker names.
fn queue_all(handle: &PushHandle<Envelope>, frames: &[LaneFrame]) -> Result<(), PushError> {
    for (is_high, marker) in frames {
        let priority = if *is_high {
            PushPriority::High
        } else {
            PushPriority::Low
        };
        handle.try_push(
            Envelope::new(0, None, vec![*marker]),
            priority,
            PushPolicy::ReturnErrorIfFull,
        )?;
    }
    Ok(())
}

/// The marker byte a queued frame carries.
///
/// Generated markers are `0..=24`, so `u8::MAX` can only mean an unexpectedly
/// empty payload, and that cannot match a queued marker.
fn frame_marker(frame: &Envelope) -> u8 {
    frame.payload_bytes().first().copied().unwrap_or(u8::MAX)
}

/// Collect every queued frame through the actor's normal delivery path.
///
/// Every wait is bounded: the generated case fails when the actor stops
/// delivering while the lanes it still polls are empty, rather than blocking
/// the test binary.
fn collect_from_actor(
    actor: &mut ConnectionActor<Envelope, ()>,
    expected: usize,
) -> Result<Vec<u8>, TestCaseError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| TestCaseError::fail(format!("the runtime must build: {error}")))?;
    runtime.block_on(async {
        let mut delivered = Vec::with_capacity(expected);
        while delivered.len() < expected {
            let next = tokio::time::timeout(COLLECTION_TIMEOUT, actor.next_pushes())
                .await
                .map_err(|_| {
                    TestCaseError::fail(
                        "the actor stopped delivering a queued frame while its lanes stayed open",
                    )
                })?;
            let Some(batch) = next else {
                break;
            };
            delivered.extend(batch.frames.iter().map(frame_marker));
        }
        Ok(delivered)
    })
}

/// The order the fairness policy requires for one lane interleaving.
///
/// Urgent frames go first in bursts of [`FAIRNESS_THRESHOLD`], and one
/// best-effort frame follows each full burst while that lane still has work.
/// Once a lane is exhausted the remaining frames of the other lane follow in
/// their queue order.
fn expected_fair_order(frames: &[LaneFrame]) -> Vec<u8> {
    let urgent = lane_markers(frames, true);
    let best_effort = lane_markers(frames, false);
    let mut expected = Vec::with_capacity(frames.len());
    let (mut next_urgent, mut next_best_effort) = (0, 0);
    while next_urgent < urgent.len() || next_best_effort < best_effort.len() {
        let burst_end = (next_urgent + FAIRNESS_THRESHOLD).min(urgent.len());
        let burst = urgent.get(next_urgent..burst_end).unwrap_or_default();
        expected.extend_from_slice(burst);
        next_urgent = burst_end;
        if let Some(marker) = best_effort.get(next_best_effort) {
            expected.push(*marker);
            next_best_effort += 1;
        }
    }
    expected
}

/// Markers of one lane, in the order they were queued.
fn lane_markers(frames: &[LaneFrame], is_high: bool) -> Vec<u8> {
    frames
        .iter()
        .filter(|(lane, _)| *lane == is_high)
        .map(|(_, marker)| *marker)
        .collect()
}

// With fewer urgent frames than the fairness threshold, every urgent frame is
// delivered before the first best-effort one, and each lane keeps its queue
// order.
proptest! {
    #[test]
    fn below_the_fairness_threshold_urgent_frames_go_first(
        lanes in proptest::collection::vec(any::<bool>(), 0..=14),
    ) {
        prop_assume!(lanes.iter().filter(|is_high| **is_high).count() < FAIRNESS_THRESHOLD);
        let frames: Vec<LaneFrame> = lanes
            .iter()
            .enumerate()
            .map(|(index, is_high)| (*is_high, u8::try_from(index).unwrap_or(u8::MAX)))
            .collect();
        let (mut actor, handle) = actor_with_lanes()?;
        queue_all(&handle, &frames).map_err(|error| TestCaseError::fail(format!("queue: {error}")))?;

        let mut expected = lane_markers(&frames, true);
        expected.extend(lane_markers(&frames, false));
        prop_assert_eq!(collect_from_actor(&mut actor, frames.len())?, expected);
    }
}

// Once the urgent burst reaches the fairness threshold, a best-effort frame may
// be pulled forward. Whatever the interleaving, no frame is lost, each lane
// keeps its order, and a best-effort frame that overtakes an urgent one does so
// only after a full urgent burst.
proptest! {
    #[test]
    fn any_interleaving_preserves_lane_order(
        lanes in proptest::collection::vec(any::<bool>(), 0..=24),
    ) {
        let frames: Vec<LaneFrame> = lanes
            .iter()
            .enumerate()
            .map(|(index, is_high)| (*is_high, u8::try_from(index).unwrap_or(u8::MAX)))
            .collect();
        let (mut actor, handle) = actor_with_lanes()?;
        queue_all(&handle, &frames).map_err(|error| TestCaseError::fail(format!("queue: {error}")))?;

        let output = collect_from_actor(&mut actor, frames.len())?;
        prop_assert_eq!(output.len(), frames.len(), "every queued frame is delivered once");

        let mut sorted = output.clone();
        sorted.sort_unstable();
        let mut queued: Vec<u8> = frames.iter().map(|(_, marker)| *marker).collect();
        queued.sort_unstable();
        prop_assert_eq!(sorted, queued, "no frame may be duplicated or dropped");

        for is_high in [true, false] {
            let expected = lane_markers(&frames, is_high);
            let observed: Vec<u8> = output
                .iter()
                .copied()
                .filter(|marker| expected.contains(marker))
                .collect();
            prop_assert_eq!(observed, expected, "one lane must keep its queue order");
        }

        prop_assert_eq!(
            output,
            expected_fair_order(&frames),
            "each full urgent burst must be followed by exactly one best-effort frame while that \
             lane has work"
        );
    }
}

/// A fairness yield makes the batch mixed rather than urgent.
///
/// The batch carries no single priority once it holds frames from both lanes,
/// so a writer that abandons one of them cannot report the wrong lane.
#[test]
fn a_fairness_yield_reports_a_mixed_batch() {
    let (mut actor, handle) = actor_with_lanes().expect("the actor should build");
    let threshold =
        u8::try_from(FAIRNESS_THRESHOLD).expect("the fairness threshold fits a frame marker");
    let mut frames: Vec<LaneFrame> = (0..threshold).map(|m| (true, m)).collect();
    frames.push((false, threshold));
    queue_all(&handle, &frames).expect("the lanes have room for every frame");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("the runtime should build");
    for burst in 1..=FAIRNESS_THRESHOLD {
        let batch = runtime
            .block_on(actor.next_pushes())
            .expect("every queued frame is delivered");
        if burst < FAIRNESS_THRESHOLD {
            assert_eq!(
                batch.priority,
                Some(PushPriority::High),
                "a batch from one lane reports that lane"
            );
        } else {
            assert_eq!(
                batch.priority, None,
                "the batch a fairness yield extends must report no single lane"
            );
            assert_eq!(
                batch.frames.len(),
                2,
                "the yield appends one best-effort frame"
            );
        }
    }
}

// A close-time drain delivers every queued frame, urgent lane first, and then
// refuses new pushes on both lanes rather than accepting a frame the actor will
// never poll again.
proptest! {
    #[test]
    fn draining_at_close_orders_by_priority_and_refuses_later_pushes(
        lanes in proptest::collection::vec(any::<bool>(), 0..=24),
    ) {
        let frames: Vec<LaneFrame> = lanes
            .iter()
            .enumerate()
            .map(|(index, is_high)| (*is_high, u8::try_from(index).unwrap_or(u8::MAX)))
            .collect();
        let (mut actor, handle) = actor_with_lanes()?;
        queue_all(&handle, &frames).map_err(|error| TestCaseError::fail(format!("queue: {error}")))?;

        let pending = actor.take_pending_pushes();
        let mut expected = lane_markers(&frames, true);
        expected.extend(lane_markers(&frames, false));
        let drained: Vec<u8> = pending.frames.iter().map(frame_marker).collect();
        prop_assert_eq!(drained, expected);

        for priority in [PushPriority::High, PushPriority::Low] {
            prop_assert_eq!(
                handle.try_push(
                    Envelope::new(0, None, vec![u8::MAX]),
                    priority,
                    PushPolicy::ReturnErrorIfFull,
                ),
                Err(PushError::Closed),
                "a drained lane must refuse a push rather than silently drop it"
            );
        }
    }
}
