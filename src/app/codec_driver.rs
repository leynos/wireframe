//! Codec-aware connection driver that bridges the connection actor's frame
//! processing pipeline to a framed transport stream.
//!
//! The [`FramePipeline`] applies optional fragmentation and outbound metrics
//! to every [`Envelope`] produced by an application handler. The
//! [`send_envelope`] and [`flush_pipeline_output`] helpers then serialize,
//! wrap via [`FrameCodec::wrap_payload`], run the configured protocol
//! `before_send` hook, and write the resulting frame to the underlying
//! [`Framed`] stream.
//!
//! Push messages and multi-packet channel frames are emitted by the
//! `ConnectionActor`, which runs its own hook pipeline; only application
//! handler responses pass through this module.

use futures::SinkExt;
use log::warn;
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use super::{
    combined_codec::ConnectionCodec,
    envelope::Envelope,
    fragmentation_state::FragmentationState,
    outbound_encoding::encode_message_frame,
};
use crate::{
    codec::{CodecError, FrameCodec},
    fragment::{FragmentationConfig, FragmentationError},
    hooks::{ConnectionContext, ProtocolHooks},
    message::EncodeWith,
    metrics::{AppResponseSendOutcome, inc_app_response_send},
    serializer::Serializer,
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
    fn before_send(&mut self, frame: &mut T) { self.hooks.before_send(frame, &mut self.context); }

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

/// Outbound transport parameters shared by the frame-sending helpers.
///
/// The serializer, codec, and framed stream always travel together, so
/// grouping them keeps the send helpers within the argument limits the
/// project lints enforce.
pub(super) struct OutboundTransport<'a, S, W, F>
where
    F: FrameCodec,
{
    /// Serializer converting an envelope into payload bytes.
    pub(crate) serializer: &'a S,
    /// Frame codec wrapping payloads into wire frames.
    pub(crate) codec: &'a F,
    /// Framed stream receiving encoded frames.
    pub(crate) framed: &'a mut Framed<W, ConnectionCodec<F>>,
}

/// Serialize an [`Envelope`], run the `before_send` hook, and write the
/// resulting frame through the codec to the framed stream.
///
/// The hook ordering matches the connection actor: the frame is serialized
/// and wrapped by the codec first, so protocol implementations receive the
/// exact frame that will be written, and any mutation they apply reaches the
/// wire.
///
/// # Errors
///
/// Returns an [`io::Error`] if serialization or sending fails.
pub(super) async fn send_envelope<S, W, F>(
    transport: &mut OutboundTransport<'_, S, W, F>,
    envelope: &Envelope,
    pipeline: &mut FramePipeline<F::Frame>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let mut encoded = encode_message_frame(transport.serializer, transport.codec, envelope)
        .map_err(|e| {
            let id = envelope.id;
            let correlation_id = envelope.correlation_id;
            warn!(
                "failed to serialize outbound envelope: stage=serialize, \
                 category=serialization_failure, id={id}, correlation_id={correlation_id:?}, \
                 error={e:?}"
            );
            crate::metrics::inc_handler_errors();
            inc_app_response_send(AppResponseSendOutcome::SerializationFailure);
            io::Error::other(e)
        })?;
    pipeline.before_send(&mut encoded.frame);
    match transport.framed.send(encoded.frame).await {
        Ok(()) => {
            inc_app_response_send(AppResponseSendOutcome::Success);
            Ok(())
        }
        Err(error) => {
            let id = envelope.id;
            let correlation_id = envelope.correlation_id;
            let outcome = classify_send_error(&error);
            warn!(
                "failed to send outbound frame: stage=write, category={}, id={id}, \
                 correlation_id={correlation_id:?}, error={error:?}",
                outcome.as_str()
            );
            crate::metrics::inc_handler_errors();
            inc_app_response_send(outcome);
            Err(io::Error::other(error))
        }
    }
}

/// Classify an outbound write failure for the bounded send-outcome metric.
///
/// The default [`FrameCodec`] reports an oversized hook result as a
/// [`CodecError`] wrapped inside the I/O error, which separates a codec
/// rejection from a transport failure. A custom encoder that reports the same
/// condition as a plain [`io::Error`] is counted as a transport failure; the
/// counter stays bounded either way.
fn classify_send_error(error: &io::Error) -> AppResponseSendOutcome {
    let is_codec_rejection = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<CodecError>())
        .is_some_and(|codec_error| matches!(codec_error, CodecError::Framing(_)));
    if is_codec_rejection {
        AppResponseSendOutcome::CodecRejected
    } else {
        AppResponseSendOutcome::TransportFailure
    }
}

/// Flush a batch of pipeline-produced [`Envelope`] values through the codec
/// to the framed stream.
///
/// Each envelope is serialized, wrapped, passed through the pipeline's
/// `before_send` hook, and written individually. On the first I/O failure the
/// remaining envelopes are discarded and the error is returned.
///
/// # Errors
///
/// Returns an [`io::Error`] if any envelope fails to serialize or send.
pub(super) async fn flush_pipeline_output<S, W, F>(
    transport: &mut OutboundTransport<'_, S, W, F>,
    pipeline: &mut FramePipeline<F::Frame>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let mut envelopes = pipeline.drain_output();
    for envelope in envelopes.drain(..) {
        send_envelope(transport, &envelope, pipeline).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "codec_driver_tests.rs"]
mod tests;
