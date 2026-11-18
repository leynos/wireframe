//! Property coverage for push-lane ordering and close-time draining.
//!
//! The actor polls urgent frames before best-effort ones, yields to the low
//! lane only after a configured burst, and takes both lanes in priority order
//! when the peer stops writing. These properties generate interleavings of the
//! two lanes rather than replaying a fixed example, so a change to the biased
//! select, the fairness threshold, or the drain order cannot satisfy them by
//! accident.

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

/// Collect every queued frame through the actor's normal delivery path.
fn collect_from_actor(actor: &mut ConnectionActor<Envelope, ()>, expected: usize) -> Vec<u8> {
    futures::executor::block_on(async {
        let mut delivered = Vec::with_capacity(expected);
        while delivered.len() < expected {
            let Some(batch) = actor.next_pushes().await else {
                break;
            };
            delivered.extend(batch.frames.iter().map(|frame| frame.payload_bytes()[0]));
        }
        delivered
    })
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
        prop_assert_eq!(collect_from_actor(&mut actor, frames.len()), expected);
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

        let output = collect_from_actor(&mut actor, frames.len());
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

        assert_overtaking_needs_a_burst(&frames, &output)?;
    }
}

/// Check that a best-effort frame overtakes an urgent one only after a burst.
///
/// Returns a failure when a best-effort frame appears ahead of a later urgent
/// frame while fewer than [`FAIRNESS_THRESHOLD`] urgent frames have been
/// delivered before it: that is the ordering guarantee, not a fairness yield.
fn assert_overtaking_needs_a_burst(
    frames: &[LaneFrame],
    delivered: &[u8],
) -> Result<(), TestCaseError> {
    let urgent = lane_markers(frames, true);
    let best_effort = lane_markers(frames, false);
    for (position, marker) in delivered.iter().enumerate() {
        if !best_effort.contains(marker) {
            continue;
        }
        let urgent_after = delivered[position + 1..]
            .iter()
            .any(|later| urgent.contains(later));
        if !urgent_after {
            continue;
        }
        let urgent_before = delivered[..position]
            .iter()
            .filter(|seen| urgent.contains(seen))
            .count();
        prop_assert!(
            urgent_before >= FAIRNESS_THRESHOLD,
            "best-effort frame {marker} overtook urgent traffic after only {urgent_before} urgent \
             frames"
        );
    }
    Ok(())
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

        let pending = futures::executor::block_on(actor.take_pending_pushes());
        let mut expected = lane_markers(&frames, true);
        expected.extend(lane_markers(&frames, false));
        let drained: Vec<u8> = pending
            .frames
            .iter()
            .map(|frame| frame.payload_bytes()[0])
            .collect();
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
