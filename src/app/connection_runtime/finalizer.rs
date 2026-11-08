//! At-most-once connection finalization state.

use std::sync::Arc;

use crate::app::lifecycle::ConnectionTeardown;

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
    pub(super) async fn finalize(&mut self, teardown: Option<&Arc<ConnectionTeardown<C>>>) {
        let Some(state) = self.take_for_teardown() else {
            return;
        };
        if let Some(callback) = teardown {
            callback(state).await;
        }
    }
}
