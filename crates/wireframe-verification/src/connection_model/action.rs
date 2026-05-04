//! Actions in the placeholder connection-actor model.

/// Nondeterministic events for the placeholder connection model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConnectionAction {
    /// Enqueue a high-priority output request. Skipped if one is already queued.
    EnqueueHigh,
    /// Enqueue a low-priority output request. Skipped if one is already queued.
    EnqueueLow,
    /// Install a response stream as the active output when the output is idle
    /// and shutdown has not been requested.
    InstallResponse,
    /// Install a multi-packet stream as the active output, resetting the
    /// terminator count, when the output is idle and shutdown has not been
    /// requested.
    InstallMultiPacket,
    /// Emit the highest-priority queued output (high before low, respecting
    /// fairness) and dequeue it.
    EmitQueued,
    /// Emit a frame from the currently active output stream, recording whether
    /// shutdown was racing.
    EmitActiveFrame,
    /// Complete the active output stream, returning to idle and recording
    /// completion for the response or multi-packet case.
    CompleteActiveOutput,
    /// Request shutdown, recording whether it races an active output.
    Shutdown,
    /// Allow a waiting low-priority output to proceed (fairness tick).
    TickFairness,
}
