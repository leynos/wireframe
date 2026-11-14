//! At-most-once connection finalization state.

use std::{
    panic::{AssertUnwindSafe, resume_unwind},
    sync::Arc,
};

use futures::FutureExt;
use tracing::{debug, error};

use crate::app::lifecycle::ConnectionTeardown;

/// Why the connection stopped processing, as reported to teardown diagnostics.
///
/// The category is a bounded, static label. It never carries the panic payload,
/// the setup state, or peer data, so it is safe to log on every connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FinalizationOutcome {
    /// Processing reached a clean end of stream.
    Clean,
    /// Processing returned an error, such as a decode or transport failure.
    ReturnedError,
    /// Processing panicked and the runtime is finalizing before re-raising.
    HandlerPanic,
}

impl FinalizationOutcome {
    /// Stable label used in structured diagnostics.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::ReturnedError => "returned_error",
            Self::HandlerPanic => "handler_panic",
        }
    }
}

/// Tracks whether setup produced state that still owes a teardown callback.
///
/// This is deliberately not a `Drop` implementation. Teardown callbacks are
/// asynchronous, and a `Drop` implementation cannot await them; running user
/// application I/O from `Drop` is a non-goal of the connection-runtime
/// extraction. Instead the runtime arms this value after setup succeeds and
/// takes the state itself before awaiting teardown.
///
/// The three states make the lifecycle contract explicit:
///
/// - [`NotSetUp`](Self::NotSetUp) when no setup hook was registered;
/// - [`Armed`](Self::Armed) once setup produced connection state `C`; and
/// - [`Finalized`](Self::Finalized) once the state has been taken for teardown.
///
/// Taking the state replaces the finalizer with `Finalized` *before* the
/// callback is invoked, so a teardown panic cannot cause a second teardown.
pub(super) enum ConnectionFinalizer<C> {
    /// No setup hook ran, so no teardown is owed.
    NotSetUp,
    /// Setup produced state and teardown is owed exactly once.
    Armed(C),
    /// The setup state has been taken; no further teardown may run.
    Finalized,
}

impl<C> ConnectionFinalizer<C> {
    /// Construct a finalizer that owes no teardown.
    pub(super) const fn not_set_up() -> Self { Self::NotSetUp }

    /// Move setup state into the finalizer.
    ///
    /// Only `NotSetUp` can be armed. A call from any other state is ignored
    /// and drops `state`, which preserves the at-most-once invariant even if
    /// the driver is ever restructured.
    pub(super) fn arm(&mut self, state: C) {
        if matches!(self, Self::NotSetUp) {
            *self = Self::Armed(state);
        }
    }

    /// Take the setup value so teardown can receive it by value.
    ///
    /// The finalizer is replaced with `Finalized` before the value is
    /// returned, so a second call returns `None` even if the teardown callback
    /// panics after this point.
    pub(super) fn take_for_teardown(&mut self) -> Option<C> {
        match std::mem::replace(self, Self::Finalized) {
            Self::Armed(state) => Some(state),
            Self::NotSetUp | Self::Finalized => None,
        }
    }

    /// Run the registered teardown callback at most once.
    ///
    /// `None` means either setup never produced state or teardown already
    /// ran. A missing teardown callback still takes and drops the state, which
    /// releases connection resources without invoking user code.
    ///
    /// Start and completion are logged with the [`FinalizationOutcome`]
    /// category. A teardown panic is logged as a bounded failure and then
    /// re-raised, so supervision still observes it and it is never retried.
    pub(super) async fn finalize(
        &mut self,
        teardown: Option<&Arc<ConnectionTeardown<C>>>,
        outcome: FinalizationOutcome,
    ) {
        let Some(state) = self.take_for_teardown() else {
            return;
        };
        let Some(callback) = teardown else {
            return;
        };
        run_teardown(callback, state, outcome).await;
    }
}

/// Invoke a teardown callback once and report how it ended.
///
/// The completion event is emitted only when the callback returns, so a
/// panicking teardown never reports success. The panic is re-raised rather
/// than swallowed, which keeps supervision's existing accounting intact and
/// guarantees the callback is not retried.
async fn run_teardown<C>(
    callback: &Arc<ConnectionTeardown<C>>,
    state: C,
    outcome: FinalizationOutcome,
) {
    log_teardown_started(outcome);
    if let Err(payload) = AssertUnwindSafe(callback(state)).catch_unwind().await {
        log_teardown_panicked(outcome);
        resume_unwind(payload);
    }
    log_teardown_completed(outcome);
}

/// Report that the teardown callback is about to run.
fn log_teardown_started(outcome: FinalizationOutcome) {
    debug!(
        stage = "connection_runtime",
        operation = "teardown",
        outcome = outcome.as_str(),
        "connection teardown started"
    );
}

/// Report that the teardown callback returned normally.
fn log_teardown_completed(outcome: FinalizationOutcome) {
    debug!(
        stage = "connection_runtime",
        operation = "teardown",
        outcome = outcome.as_str(),
        "connection teardown completed"
    );
}

/// Report that the teardown callback panicked.
fn log_teardown_panicked(outcome: FinalizationOutcome) {
    error!(
        stage = "connection_runtime",
        operation = "teardown",
        outcome = outcome.as_str(),
        "connection teardown panicked; the panic is not retried"
    );
}
