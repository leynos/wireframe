//! Helpers for exercising private connection actor paths in integration tests.

// These helpers compile for all non-Loom builds so integration tests can
// exercise private connection actor paths.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    ActorState,
    ConnectionActor,
    DrainContext,
    MultiPacketTerminationReason,
    ProtocolHooks,
    QueueKind,
};
use crate::app::{Packet, PacketParts};
use crate::push::{PushConfigError, PushQueues};

impl Packet for u8 {
    fn id(&self) -> u32 { 0 }

    fn correlation_id(&self) -> Option<u64> { None }

    fn into_parts(self) -> PacketParts { PacketParts::new(0, None, vec![self]) }

    fn from_parts(parts: PacketParts) -> Self {
        parts.payload().first().copied().unwrap_or_default()
    }
}

impl Packet for Vec<u8> {
    fn id(&self) -> u32 { 0 }

    fn correlation_id(&self) -> Option<u64> { None }

    fn into_parts(self) -> PacketParts { PacketParts::new(0, None, self) }

    fn from_parts(parts: PacketParts) -> Self { parts.payload() }
}

/// Build a connection actor configured with the supplied protocol hooks.
///
/// # Errors
///
/// Returns an error if the push queues cannot be constructed.
pub fn create_test_actor_with_hooks(
    hooks: ProtocolHooks<u8, ()>,
) -> Result<ConnectionActor<u8, ()>, PushConfigError> {
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .build()?;
    Ok(ConnectionActor::with_hooks(
        queues,
        handle,
        None,
        CancellationToken::new(),
        hooks,
    ))
}

/// Convenience harness wrapping an actor, its state, and buffered output.
pub struct ActorHarness {
    actor: ConnectionActor<u8, ()>,
    state: ActorState,
    pub out: Vec<u8>,
}

impl Default for ActorHarness {
    fn default() -> Self { Self::new().expect("failed to build ActorHarness") }
}

impl ActorHarness {
    /// Create a harness with custom hooks and state flags.
    ///
    /// # Errors
    ///
    /// Returns an error if the push queues cannot be constructed.
    pub fn new_with_state(
        hooks: ProtocolHooks<u8, ()>,
        has_response: bool,
        has_multi_packet: bool,
    ) -> Result<Self, PushConfigError> {
        let actor = create_test_actor_with_hooks(hooks)?;
        Ok(Self {
            actor,
            state: ActorState::new(has_response, has_multi_packet),
            out: Vec::new(),
        })
    }

    /// Create a harness using default hooks and no active streams.
    ///
    /// # Errors
    ///
    /// Returns an error if the push queues cannot be constructed.
    pub fn new() -> Result<Self, PushConfigError> {
        Self::new_with_state(ProtocolHooks::<u8, ()>::default(), false, false)
    }

    /// Snapshot the internal actor state.
    #[must_use]
    pub fn snapshot(&self) -> ActorStateSnapshot {
        ActorStateSnapshot {
            is_active: self.state.is_active(),
            is_shutting_down: self.state.is_shutting_down(),
            is_done: self.state.is_done(),
            total_sources: self.state.total_sources,
            closed_sources: self.state.closed_sources,
        }
    }
    /// Replace the low-priority receiver.
    pub fn set_low_queue(&mut self, queue: Option<mpsc::Receiver<u8>>) {
        self.actor.set_low_queue(queue);
    }

    /// Replace the multi-packet receiver.
    pub fn set_multi_queue(&mut self, queue: Option<mpsc::Receiver<u8>>) {
        self.actor.set_multi_packet(queue);
    }

    /// Returns `true` when the low-priority queue is still available.
    #[must_use]
    pub fn has_low_queue(&self) -> bool { self.actor.low_rx.is_some() }

    /// Returns `true` when the multi-packet queue is still available.
    #[must_use]
    pub fn has_multi_queue(&self) -> bool { self.actor.multi_packet.is_active() }

    /// Process a multi-packet poll result.
    pub fn process_multi_packet(&mut self, res: Option<u8>) {
        self.actor.process_queue(
            QueueKind::Multi,
            res,
            DrainContext {
                out: &mut self.out,
                state: &mut self.state,
            },
        );
    }

    /// Handle closure of the multi-packet receiver.
    pub fn handle_multi_packet_closed(&mut self) {
        self.actor.handle_multi_packet_closed(
            MultiPacketTerminationReason::Drained,
            &mut self.state,
            &mut self.out,
        );
    }

    /// Trigger shutdown handling on the underlying actor.
    pub fn start_shutdown(&mut self) { self.actor.start_shutdown(&mut self.state); }

    /// Attempt a low-priority opportunistic drain.
    pub fn try_drain_low(&mut self) -> bool {
        let state = &mut self.state;
        let out = &mut self.out;
        self.actor
            .try_opportunistic_drain(QueueKind::Low, DrainContext { out, state })
    }

    /// Attempt a multi-packet opportunistic drain.
    pub fn try_drain_multi(&mut self) -> bool {
        let state = &mut self.state;
        let out = &mut self.out;
        self.actor
            .try_opportunistic_drain(QueueKind::Multi, DrainContext { out, state })
    }

    /// Access the underlying actor mutably.
    pub fn actor_mut(&mut self) -> &mut ConnectionActor<u8, ()> { &mut self.actor }
}

/// Snapshot of the actor lifecycle flags and counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorStateSnapshot {
    pub is_active: bool,
    pub is_shutting_down: bool,
    pub is_done: bool,
    pub total_sources: usize,
    pub closed_sources: usize,
}

/// Harness around `ActorState` for integration tests.
pub struct ActorStateHarness {
    state: ActorState,
}

impl ActorStateHarness {
    /// Construct a harness with the provided active sources.
    #[must_use]
    pub fn new(has_response: bool, has_multi_packet: bool) -> Self {
        Self {
            state: ActorState::new(has_response, has_multi_packet),
        }
    }

    /// Mark a source as closed.
    pub fn mark_closed(&mut self) { self.state.mark_closed(); }

    /// Observe the current state snapshot.
    #[must_use]
    pub fn snapshot(&self) -> ActorStateSnapshot {
        ActorStateSnapshot {
            is_active: self.state.is_active(),
            is_shutting_down: self.state.is_shutting_down(),
            is_done: self.state.is_done(),
            total_sources: self.state.total_sources,
            closed_sources: self.state.closed_sources,
        }
    }
}

/// Await a frame from the provided queue, returning `None` when absent.
pub async fn poll_queue_next(rx: Option<&mut mpsc::Receiver<u8>>) -> Option<u8> {
    ConnectionActor::<u8, ()>::poll_queue(rx).await
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;

    #[test]
    fn has_multi_queue_false_by_default() {
        let harness = ActorHarness::new().expect("build ActorHarness");
        assert!(
            !harness.has_multi_queue(),
            "multi-packet queue should start inactive"
        );
    }

    #[test]
    fn has_multi_queue_true_after_install() {
        let mut harness = ActorHarness::new().expect("build ActorHarness");
        let (_tx, rx) = mpsc::channel(1);
        harness.set_multi_queue(Some(rx));
        assert!(
            harness.has_multi_queue(),
            "multi-packet queue should be active after install"
        );
    }

    #[test]
    fn has_multi_queue_false_after_clear() {
        let mut harness = ActorHarness::new().expect("build ActorHarness");
        let (_tx, rx) = mpsc::channel(1);
        harness.set_multi_queue(Some(rx));
        assert!(
            harness.has_multi_queue(),
            "multi-packet queue should be active after install"
        );
        harness.set_multi_queue(None);
        assert!(
            !harness.has_multi_queue(),
            "multi-packet queue should be inactive after clear"
        );
    }
}
