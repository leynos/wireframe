//! Actor lifecycle state management.

/// Internal run state for the connection actor.
pub(super) enum RunState {
    /// All sources are open and frames are still being processed.
    Active,
    /// A shutdown request has been observed and queues are being closed.
    ShuttingDown,
    /// All sources have completed and the actor can exit.
    Finished,
}

/// Tracks progress through the actor lifecycle.
pub(super) struct ActorState {
    run_state: RunState,
    pub(super) closed_sources: usize,
    pub(super) total_sources: usize,
}

impl ActorState {
    /// Create a new `ActorState`.
    ///
    /// `has_response` indicates whether a streaming response is currently
    /// attached.
    /// `has_multi_packet` signals that a channel-backed response is active.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use wireframe::connection::ActorState;
    ///
    /// let state = ActorState::new(true, false);
    /// assert!(state.is_active());
    /// ```
    pub(super) fn new(has_response: bool, has_multi_packet: bool) -> Self {
        Self {
            run_state: RunState::Active,
            // The shutdown token is considered closed until cancellation
            // occurs, matching previous behaviour where draining sources
            // without explicit shutdown terminates the actor.
            closed_sources: 1,
            // total_sources counts all sources that keep the actor alive:
            // - 3 for the baseline sources (main loop, shutdown token, and queue drains)
            // - +1 if a streaming response is active (has_response)
            // - +1 if multi-packet handling is enabled (has_multi_packet)
            total_sources: 3 + usize::from(has_response) + usize::from(has_multi_packet),
        }
    }

    /// Mark a source as closed and update the run state if all are closed.
    pub(super) fn mark_closed(&mut self) {
        self.closed_sources += 1;
        if self.closed_sources >= self.total_sources {
            self.run_state = RunState::Finished;
        }
    }

    /// Transition to `ShuttingDown` if currently active.
    pub(super) fn start_shutdown(&mut self) {
        if matches!(self.run_state, RunState::Active) {
            self.run_state = RunState::ShuttingDown;
        }
    }

    /// Returns `true` while the actor is actively processing sources.
    pub(super) fn is_active(&self) -> bool { matches!(self.run_state, RunState::Active) }

    /// Returns `true` once shutdown has begun.
    pub(super) fn is_shutting_down(&self) -> bool {
        matches!(self.run_state, RunState::ShuttingDown)
    }

    /// Returns `true` when all sources have finished.
    pub(super) fn is_done(&self) -> bool { matches!(self.run_state, RunState::Finished) }
}
