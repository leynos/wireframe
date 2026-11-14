//! Mutators that install the actor's output sources and runtime policy.
//!
//! These setters are generic over the push frame type so the server can drive
//! an actor whose queued frames differ from its response frames. They live
//! beside the actor definition rather than inside it to keep each module
//! focused and within the project's file-size budget.

#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use super::grace::GraceTimer;
use super::{
    ActiveOutput,
    ConnectionActor,
    ConnectionStateError,
    FairnessConfig,
    multi_packet::MultiPacketContext,
};
use crate::{
    app::Packet,
    correlation::CorrelatableFrame,
    fragment::{FragmentationConfig, Fragmenter},
    push::FrameLike,
    response::FrameStream,
};

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Set or replace the current streaming response.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionStateError::MultiPacketActive`] if a multi-packet
    /// channel is currently active.
    pub fn set_response(
        &mut self,
        stream: Option<FrameStream<F, E>>,
    ) -> Result<(), ConnectionStateError> {
        if self.active_output.is_multi_packet() {
            return Err(ConnectionStateError::MultiPacketActive);
        }
        self.active_output = stream.map_or_else(|| ActiveOutput::None, ActiveOutput::Response);
        Ok(())
    }

    /// Set or replace the current multi-packet response channel.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionStateError::ResponseActive`] if a response stream is
    /// currently active.
    pub fn set_multi_packet(
        &mut self,
        channel: Option<mpsc::Receiver<F>>,
    ) -> Result<(), ConnectionStateError> {
        self.set_multi_packet_with_correlation(channel, None)
    }

    /// Set or replace the current multi-packet response channel and stamp correlation identifiers.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectionStateError::ResponseActive`] if a response stream is
    /// currently active.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use tokio::sync::mpsc;
    /// # use tokio_util::sync::CancellationToken;
    /// # use wireframe::{connection::ConnectionActor, push::PushQueues};
    /// # let (queues, handle) = PushQueues::<u8>::builder()
    /// #     .high_capacity(1)
    /// #     .low_capacity(1)
    /// #     .build()
    /// #     .expect("failed to build PushQueues");
    /// # let shutdown = CancellationToken::new();
    /// # let mut actor: ConnectionActor<u8> = ConnectionActor::new(queues, handle, None, shutdown);
    /// # let (_tx, rx) = mpsc::channel(4);
    /// actor.set_multi_packet_with_correlation(Some(rx), Some(7))?;
    /// # Ok::<(), wireframe::connection::ConnectionStateError>(())
    /// ```
    pub fn set_multi_packet_with_correlation(
        &mut self,
        channel: Option<mpsc::Receiver<F>>,
        correlation_id: Option<u64>,
    ) -> Result<(), ConnectionStateError> {
        if self.active_output.is_response() {
            return Err(ConnectionStateError::ResponseActive);
        }
        self.active_output = channel.map_or_else(
            || ActiveOutput::None,
            |rx| {
                let mut ctx = MultiPacketContext::new();
                ctx.install(Some(rx), correlation_id);
                ActiveOutput::MultiPacket(ctx)
            },
        );
        Ok(())
    }
}

impl<F, E, P> ConnectionActor<F, E, P> {
    /// Replace the low-priority queue used for tests.
    pub fn set_low_queue(&mut self, queue: Option<mpsc::Receiver<P>>) { self.low_rx = queue; }

    /// Get a clone of the shutdown token used by the actor.
    #[must_use]
    pub fn shutdown_token(&self) -> CancellationToken { self.shutdown.clone() }
}

impl<F, E, P> ConnectionActor<F, E, P>
where
    F: FrameLike + CorrelatableFrame + Packet,
    E: std::fmt::Debug,
    P: FrameLike,
{
    /// Replace the fairness configuration.
    pub const fn set_fairness(&mut self, fairness: FairnessConfig) {
        self.fairness.set_config(fairness);
    }

    /// Enable transparent fragmentation for outbound frames.
    ///
    /// When configured, frames that exceed `fragment_payload_cap` are split
    /// into multiple fragments carrying a standard fragment header inside the
    /// payload. Callers continue to enqueue complete frames; fragmentation
    /// occurs just before hooks and metrics are applied.
    pub const fn enable_fragmentation(&mut self, config: FragmentationConfig) {
        self.fragmenter = Some(Fragmenter::new(config.fragment_payload_cap));
    }

    /// Override the idle grace used once the peer can no longer be observed.
    ///
    /// The server keeps [`PEER_CLOSED_RESPONSE_GRACE`]; tests pass a short
    /// grace and a paused clock to prove that a stalled response is cancelled
    /// without waiting for the production deadline.
    ///
    /// [`PEER_CLOSED_RESPONSE_GRACE`]: super::PEER_CLOSED_RESPONSE_GRACE
    pub(crate) const fn set_peer_closed_grace(&mut self, grace: Duration) {
        self.peer_closed_grace = grace;
    }

    /// Replace the deadline source that enforces the idle grace.
    ///
    /// Production keeps the ambient Tokio clock. A test installs a deadline it
    /// fires by hand, so cancellation is proved to happen when the grace
    /// expires rather than merely eventually.
    #[cfg(test)]
    pub(crate) fn set_grace_timer(&mut self, timer: Arc<dyn GraceTimer>) {
        self.grace_timer = timer;
    }
}
