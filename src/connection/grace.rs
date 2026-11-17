//! Deadline source for the response idle grace.
//!
//! The driver bounds a response whose peer can no longer be observed, but a
//! test must not wait for the production interval. The actor holds a
//! [`GraceTimer`], so production keeps the ambient Tokio clock while a test
//! installs a manually fired deadline and proves that cancellation happens
//! exactly when the grace expires, and not before.

use std::{future::Future, pin::Pin, time::Duration};

use log::warn;

use super::{ConnectionActor, UnobservedCause};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    metrics::ResponseCancellationReason,
    push::FrameLike,
};

/// Future a [`GraceTimer`] hands back for one deadline.
///
/// The future is owned so the actor can hold it across a `select!` without
/// borrowing the timer, which keeps the driver's borrow of the actor simple.
pub(crate) type GraceFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Supplies the deadline that bounds an unobserved response.
pub(crate) trait GraceTimer: Send + Sync {
    /// Resolve once `grace` has elapsed.
    fn elapse(&self, grace: Duration) -> GraceFuture;
}

/// Timer backed by the ambient Tokio clock.
///
/// Production connections use this timer. Tests replace it with a manually
/// fired deadline, so a paused clock is not needed to exercise cancellation.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TokioGraceTimer;

impl GraceTimer for TokioGraceTimer {
    fn elapse(&self, grace: Duration) -> GraceFuture { Box::pin(tokio::time::sleep(grace)) }
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
{
    /// Abandon the active response because the idle grace expired.
    ///
    /// The response is cleared rather than drained: a peer that has neither
    /// accepted a frame nor been observed for a whole grace interval is treated
    /// as gone, and any frame it later offers is dropped with the connection.
    /// The same applies to the protocol terminator, which is written under this
    /// bound.
    pub(super) fn cancel_stalled_response(&mut self) {
        warn!(
            "the peer could not be observed ({}) and accepted no frame for {:?}; cancelling: \
             id={:?}, peer={:?}, correlation={:?}",
            self.unobserved_cause_label(),
            self.peer_closed_grace,
            self.connection_id,
            self.peer_addr,
            self.response_correlation
        );
        ResponseCancellationReason::IdleGraceExpired.record();
        self.active_output.clear();
    }

    /// Name why direct peer observation stopped, when the loop recorded it.
    fn unobserved_cause_label(&self) -> &'static str {
        self.unobserved_cause
            .as_ref()
            .and_then(|cause| cause.get().copied())
            .map_or("cause unknown", UnobservedCause::as_str)
    }
}
