//! Push-queue delivery for the connection actor.
//!
//! The actor drives two frame shapes. Response frames are application
//! [`Envelope`] values that the actor fragments and hooks before a transport
//! sink serializes them. Push frames come from a connection's push queues and,
//! when the server owns the connection, are already encoded for the wire.
//!
//! [`PushComposer`] records which composition applies. A standalone actor
//! composes pushes exactly like response frames, so queue output keeps its
//! fragmentation and hook behaviour. A server-owned actor defers composition to
//! the transport sink, which serializes nothing for a push and instead runs the
//! protocol `before_send` hook on the encoded frame.
//!
//! [`Envelope`]: crate::app::Envelope

use tokio::sync::mpsc;

use super::{ConnectionActor, state::ActorState};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    push::{FrameLike, PushPriority},
};

/// Frames taken from the push lanes for one write.
///
/// The priority travels with the frames so a writer that abandons one of them
/// can name the lane it came from. A batch produced by draining both lanes has
/// no single priority and reports none.
pub(crate) struct PendingPushes<P> {
    /// Lane that produced the batch, or `None` when both lanes were drained.
    pub(crate) priority: Option<PushPriority>,
    /// Frames in delivery order, high priority first.
    pub(crate) frames: Vec<P>,
}

impl<P> PendingPushes<P> {
    /// Build a batch from frames whose lane no longer applies.
    const fn mixed(frames: Vec<P>) -> Self {
        Self {
            priority: None,
            frames,
        }
    }
}

/// Priority lane a push frame was taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PushKind {
    /// Urgent pushes polled before every other source.
    High,
    /// Best-effort pushes subject to fairness yielding.
    Low,
}

impl PushKind {
    /// Public priority this lane corresponds to.
    const fn priority(self) -> PushPriority {
        match self {
            Self::High => PushPriority::High,
            Self::Low => PushPriority::Low,
        }
    }
}

/// Composition applied to a frame taken from a push queue.
pub(super) enum PushComposer<F, E, P> {
    /// Compose pushes the same way the actor composes response frames.
    Inline(fn(&mut ConnectionActor<F, E, P>, P, &mut Vec<P>)),
    /// Hand pushes to the sink unchanged; the sink owns composition.
    Deferred,
}

impl<F, E, P> Clone for PushComposer<F, E, P> {
    fn clone(&self) -> Self { *self }
}

impl<F, E, P> Copy for PushComposer<F, E, P> {}

impl<F, E, P> PushComposer<F, E, P> {
    /// Compose one push frame into the pending buffer.
    fn compose(self, actor: &mut ConnectionActor<F, E, P>, frame: P, pushes: &mut Vec<P>) {
        match self {
            Self::Inline(compose) => compose(actor, frame, pushes),
            Self::Deferred => pushes.push(frame),
        }
    }
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Return `true` while at least one push lane can still yield frames.
    #[must_use]
    pub(crate) const fn has_open_push_lanes(&self) -> bool {
        self.high_rx.is_some() || self.low_rx.is_some()
    }

    /// Stop accepting new pushes without discarding the frames already queued.
    ///
    /// Shutdown ends the connection's willingness to take pushes. Closing the
    /// lanes wakes a producer parked on a full lane, so it learns the lane is
    /// closed instead of waiting for capacity that `stop()` has made
    /// unreachable.
    pub(crate) fn close_push_lanes(&mut self) {
        if let Some(rx) = &mut self.high_rx {
            rx.close();
        }
        if let Some(rx) = &mut self.low_rx {
            rx.close();
        }
    }

    /// Drop every push lane, discarding the frames still queued.
    ///
    /// A connection that stops while a response is still draining follows the
    /// documented rule that a frame waiting in a push queue at that moment is
    /// dropped rather than flushed. Dropping the receivers enforces that rule
    /// and wakes a producer parked on capacity with `Closed`, so it cannot keep
    /// the drain alive by feeding it another frame.
    pub(crate) fn discard_push_lanes(&mut self) {
        self.high_rx = None;
        self.low_rx = None;
    }

    /// Await the next push, returning `None` once every lane has closed.
    ///
    /// At least one frame is returned when the result is `Some`, so callers
    /// never spin on an empty batch. The biased selection keeps urgent pushes
    /// ahead of best-effort ones, and a fairness yield pulls one low-priority
    /// frame forward after a burst of urgent traffic.
    pub(crate) async fn next_pushes(&mut self) -> Option<PendingPushes<P>> {
        loop {
            if !self.has_open_push_lanes() {
                return None;
            }
            let can_poll_high = self.high_rx.is_some();
            let can_poll_low = self.low_rx.is_some();
            let high_rx = self.high_rx.as_mut();
            let low_rx = self.low_rx.as_mut();

            let (kind, received) = tokio::select! {
                biased;
                res = Self::poll_queue(high_rx), if can_poll_high => (PushKind::High, res),
                res = Self::poll_queue(low_rx), if can_poll_low => (PushKind::Low, res),
                else => return None,
            };

            let Some(frame) = received else {
                self.close_push_lane(kind);
                continue;
            };

            let mut frames = Vec::new();
            self.compose_push(frame, &mut frames);
            match kind {
                PushKind::High => self.yield_to_low_after_high(&mut frames),
                PushKind::Low => self.fairness.reset(),
            }
            return Some(PendingPushes {
                priority: Some(kind.priority()),
                frames,
            });
        }
    }

    /// Take every frame still queued when the peer stopped writing.
    ///
    /// The connection loop calls this after a clean end-of-stream so frames
    /// accepted before the close are still delivered. Frames are returned in
    /// priority order, high before low, and the caller writes them. Both lanes
    /// close before the first frame is taken, so a producer that pushes while
    /// the drain runs sees a closed channel rather than a queue that is never
    /// polled again. Either way a push attempted after this returns is rejected
    /// with [`crate::push::PushError::Closed`].
    ///
    /// Returning the frames rather than writing them lets the caller bound each
    /// write by its own idle grace: a closed lane can hold a slow producer's
    /// per-frame reservations, and one deadline for the whole drain would
    /// abandon frames the peer was still accepting.
    pub(crate) async fn take_pending_pushes(&mut self) -> PendingPushes<P> {
        // Take both lanes before reading them. A frame pushed while the drain
        // runs is then rejected by the now-closed channel instead of being
        // accepted into a lane the actor has already stopped polling, which
        // would silently lose it.
        let mut lanes = [self.high_rx.take(), self.low_rx.take()];
        // Taking a lane out of the actor does not close it: the receiver is
        // still alive in `lanes`, so a concurrent push would be accepted into a
        // lane this drain has already read and then be discarded when `lanes`
        // drops. Every lane is therefore closed before any is drained, so a
        // push arriving during the high-priority drain is rejected rather than
        // queued into a low lane that will never be read again.
        for receiver in lanes.iter_mut().flatten() {
            receiver.close();
        }
        let mut frames = Vec::new();
        for receiver in lanes.iter_mut().flatten() {
            self.drain_receiver(receiver, &mut frames).await;
        }
        PendingPushes::mixed(frames)
    }

    /// Compose one push frame according to the actor's configured routing.
    pub(super) fn compose_push(&mut self, frame: P, pushes: &mut Vec<P>) {
        let composer = self.composer;
        composer.compose(self, frame, pushes);
    }

    /// Pull one low-priority frame forward after urgent traffic.
    fn yield_to_low_after_high(&mut self, pushes: &mut Vec<P>) {
        self.fairness.record_high_priority();
        if !self.fairness.should_yield_to_low_priority() {
            return;
        }
        // The temporary state only records closure, which the caller does not
        // observe here; lane closure is handled on the next poll.
        let mut state = ActorState::new(false, false);
        self.try_opportunistic_low_drain(&mut state, pushes);
    }

    /// Drop a push lane that reported closure.
    fn close_push_lane(&mut self, kind: PushKind) {
        match kind {
            PushKind::High => self.high_rx = None,
            PushKind::Low => self.low_rx = None,
        }
    }

    /// Move every buffered frame from one closed lane into `pushes`.
    ///
    /// The lane was closed before this call, so `recv` yields `None` once the
    /// buffer is empty and every permit taken before the close has been
    /// released. Reading until then matters: `try_recv` reports `Empty` while
    /// a producer still holds a reservation, and a frame that producer then
    /// sends would be lost when the receiver drops even though the producer
    /// was told it was accepted.
    async fn drain_receiver(&mut self, receiver: &mut mpsc::Receiver<P>, pushes: &mut Vec<P>) {
        while let Some(frame) = receiver.recv().await {
            self.compose_push(frame, pushes);
        }
    }
}
