//! Codec-aware connection driver that bridges the connection actor's frame
//! processing pipeline to a framed transport stream.
//!
//! The [`FramePipeline`] applies optional fragmentation and outbound metrics
//! to every [`Envelope`] produced by an application handler. The
//! [`send_envelope`] and [`flush_pipeline_output`] helpers then serialize,
//! wrap via [`FrameCodec::wrap_payload`], run the configured protocol
//! `before_send` hook, and write the resulting frame to the connection's
//! frame sink.
//!
//! Two paths reach this module. A `ConnectionActor` driven directly through
//! `ProtocolHooks` emits push messages and multi-packet channel frames from its
//! own hook pipeline. On a prepared server connection, every handler response —
//! single, streamed, or multi-packet — is instead installed on the connection's
//! actor and written through [`ActorFrameSink`], so it shares this module's
//! pipeline and the connection's `before_send` hook. The actor built for a
//! prepared connection deliberately has no hook pipeline of its own, so a frame
//! is never processed twice.

mod outbound;

use log::warn;
pub(crate) use outbound::{ActorFrameSink, send_native_frame};
#[cfg(test)]
pub(super) use outbound::{OutboundTransport, flush_pipeline_output, send_envelope};
use tokio::io;

use super::{envelope::Envelope, fragmentation_state::FragmentationState};
use crate::{
    fragment::{FragmentationConfig, FragmentationError},
    hooks::{ConnectionContext, ProtocolHooks},
};

/// Outbound frame processing pipeline mirroring the connection actor's
/// `process_frame_with_hooks_and_metrics` logic.
///
/// Applies optional outbound envelope fragmentation and supports inbound
/// fragment reassembly.
/// Produces a buffer of processed envelopes ready for serialization and
/// transmission.
///
/// The pipeline owns the per-connection [`ProtocolHooks`] and
/// [`ConnectionContext`], so `before_send` runs exactly where the encoded
/// frame is written and connection-local hook state is retained for the
/// lifetime of the connection. Hooks are never shared between connections.
pub(crate) struct FramePipeline<T = Envelope> {
    /// State shared by outbound envelope fragmentation and inbound fragment
    /// reassembly.
    fragmentation: Option<FragmentationState>,
    /// Envelopes waiting to be serialized and written to the transport.
    out: Vec<Envelope>,
    /// Protocol hooks applied to each outbound codec frame.
    hooks: ProtocolHooks<T, ()>,
    /// Per-connection state passed to protocol hooks.
    context: ConnectionContext,
}

impl<T> FramePipeline<T> {
    /// Create a pipeline with the given optional fragmentation config and
    /// default (no-op) protocol hooks.
    ///
    /// Production connections always supply hooks via [`Self::with_hooks`];
    /// this constructor exists for test fixtures that do not exercise hooks.
    #[cfg(test)]
    pub(crate) fn new(fragmentation: Option<FragmentationConfig>) -> Self {
        Self::with_hooks(fragmentation, ProtocolHooks::default())
    }

    /// Create a pipeline with per-connection protocol hooks.
    ///
    /// The pipeline owns both the hooks and a fresh [`ConnectionContext`], so
    /// connection-local hook state never leaks into another connection. Each
    /// buffered envelope is serialized part-way through flushing; the hook
    /// then runs on the encoded frame while the frame is still in memory.
    ///
    /// ```text
    /// let hooks = ProtocolHooks {
    ///     before_send: Some(Box::new(|frame: &mut Bytes, _ctx| {
    ///         let mut bytes = frame.to_vec();
    ///         bytes.push(MARKER);
    ///         *frame = Bytes::from(bytes);
    ///     })),
    ///     ..ProtocolHooks::default()
    /// };
    /// let mut pipeline = FramePipeline::with_hooks(fragmentation, hooks);
    /// pipeline.process(envelope)?;
    /// // Every frame drained by `flush_pipeline_output` now carries the
    /// // marker appended by the hook, including each fragment.
    /// flush_pipeline_output(&mut transport, &mut pipeline).await?;
    /// ```
    pub(crate) fn with_hooks(
        fragmentation: Option<FragmentationConfig>,
        hooks: ProtocolHooks<T, ()>,
    ) -> Self {
        Self {
            fragmentation: fragmentation.map(FragmentationState::new),
            out: Vec::new(),
            hooks,
            context: ConnectionContext,
        }
    }

    /// Run the configured `before_send` hook for one outbound frame.
    pub(crate) fn before_send_frame(&mut self, frame: &mut T) {
        self.hooks.before_send(frame, &mut self.context);
    }

    /// Process an envelope through the pipeline: fragment → metrics.
    ///
    /// Processed envelopes are buffered internally. Call
    /// [`drain_output`](Self::drain_output) to retrieve them.
    pub(crate) fn process(&mut self, envelope: Envelope) -> io::Result<()> {
        let id = envelope.id;
        let correlation_id = envelope.correlation_id;
        let frames = self.fragment_envelope(envelope).map_err(|err| {
            warn!(
                "failed to fragment outbound envelope: id={id}, \
                 correlation_id={correlation_id:?}, error={err:?}"
            );
            crate::metrics::inc_handler_errors();
            io::Error::other(err)
        })?;
        for frame in frames {
            self.push_frame(frame);
        }
        Ok(())
    }

    /// Fragment an envelope if fragmentation is enabled, otherwise return it
    /// as a single-element vector.
    fn fragment_envelope(
        &mut self,
        envelope: Envelope,
    ) -> Result<Vec<Envelope>, FragmentationError> {
        match self.fragmentation.as_mut() {
            Some(state) => state.fragment(envelope),
            None => Ok(vec![envelope]),
        }
    }

    /// Return bytes retained by incomplete fragments on this connection.
    pub(crate) fn fragmentation_buffered_bytes(&self) -> usize {
        self.fragmentation
            .as_ref()
            .map_or(0, FragmentationState::buffered_bytes)
    }

    /// Purge expired fragment reassembly state, if fragmentation is enabled.
    pub(crate) fn purge_expired(&mut self) {
        if let Some(state) = self.fragmentation.as_mut() {
            state.purge_expired();
        }
    }

    /// Drain all buffered output envelopes, returning them for transmission.
    pub(crate) fn drain_output(&mut self) -> Vec<Envelope> { std::mem::take(&mut self.out) }

    /// Returns a mutable reference to the inner fragmentation state, if
    /// fragmentation is enabled.
    ///
    /// Used by the inbound reassembly path which needs direct access to
    /// [`FragmentationState::reassemble`].
    pub(crate) const fn fragmentation_mut(&mut self) -> Option<&mut FragmentationState> {
        self.fragmentation.as_mut()
    }

    /// Returns `true` when fragmentation is enabled.
    #[cfg(test)]
    pub(crate) const fn has_fragmentation(&self) -> bool { self.fragmentation.is_some() }

    /// Queue one processed envelope and account for outbound processing.
    fn push_frame(&mut self, envelope: Envelope) {
        self.out.push(envelope);
        crate::metrics::inc_frames(crate::metrics::Direction::Outbound);
    }
}

#[cfg(test)]
#[path = "../codec_driver_tests.rs"]
mod tests;
