//! Actions in the placeholder connection-actor model.

/// Nondeterministic events for the placeholder connection model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConnectionAction {
    EnqueueHigh,
    EnqueueLow,
    InstallResponse,
    InstallMultiPacket,
    EmitQueued,
    EmitActiveFrame,
    CompleteActiveOutput,
    Shutdown,
    TickFairness,
}
