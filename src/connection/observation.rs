//! Peer-observation state shared between the inbound loop and the actor.
//!
//! The inbound loop stops polling the read half once it buffers a request or
//! sees a half-close. It records why in a cell the actor reads when the idle
//! grace expires, so the cancellation warning names the cause instead of
//! leaving it in a separate diagnostic that cannot be matched to the
//! cancellation.

use std::sync::{Arc, OnceLock};

use tokio_util::sync::CancellationToken;

use super::ConnectionActor;

/// Why direct peer observation stopped for the active response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnobservedCause {
    /// The peer closed its input half.
    HalfClose,
    /// An inbound frame was buffered for the next request.
    BufferedRequest,
}

impl UnobservedCause {
    /// Describe the cause for the cancellation warning.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::HalfClose => "a half-close",
            Self::BufferedRequest => "a buffered request",
        }
    }
}

impl<F, E> ConnectionActor<F, E> {
    /// Mark the peer closed without recording why observation stopped.
    ///
    /// Tests that only need the bounded idle grace use this; the inbound loop
    /// calls [`Self::set_peer_observation`] so its warning can name the cause.
    #[cfg(test)]
    pub(crate) fn set_peer_closed(&mut self, token: CancellationToken) {
        self.set_peer_observation(token, Arc::new(OnceLock::new()));
    }

    /// Share the peer token and the cell naming why observation stopped.
    ///
    /// The inbound loop installs both before each dispatch. The cell is filled
    /// when the loop stops polling the read half, so a cancellation that
    /// follows can report whether a half-close or a buffered request caused it.
    pub(crate) fn set_peer_observation(
        &mut self,
        token: CancellationToken,
        cause: Arc<OnceLock<UnobservedCause>>,
    ) {
        self.peer_closed = token;
        self.unobserved_cause = Some(cause);
    }
}
