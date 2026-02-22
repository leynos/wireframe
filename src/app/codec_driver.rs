//! Codec-aware connection driver that bridges the connection actor's frame
//! processing pipeline to a framed transport stream.
//!
//! The [`FramePipeline`] applies optional fragmentation and outbound metrics
//! to every [`Envelope`] before it reaches the wire. The [`send_envelope`]
//! and [`flush_pipeline_output`] helpers then serialize, wrap via
//! [`FrameCodec::wrap_payload`], and write the resulting frame to the
//! underlying [`Framed`] stream.
//!
//! This module ensures all outbound frames — handler responses, push
//! messages, streaming responses, and multi-packet channels — pass through
//! the same fragmentation and metrics pipeline before reaching the wire.

use bytes::Bytes;
use futures::SinkExt;
use log::warn;
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use super::{
    combined_codec::ConnectionCodec,
    envelope::Envelope,
    fragmentation_state::FragmentationState,
};
use crate::{
    codec::FrameCodec,
    fragment::{FragmentationConfig, FragmentationError},
    message::EncodeWith,
    serializer::Serializer,
};

/// Outbound frame processing pipeline mirroring the connection actor's
/// `process_frame_with_hooks_and_metrics` logic.
///
/// Applies optional fragmentation and outbound metrics to each envelope.
/// Produces a buffer of processed envelopes ready for serialization and
/// transmission.
pub(crate) struct FramePipeline {
    fragmentation: Option<FragmentationState>,
    out: Vec<Envelope>,
}

impl FramePipeline {
    /// Create a pipeline with the given optional fragmentation config.
    pub(crate) fn new(fragmentation: Option<FragmentationConfig>) -> Self {
        Self {
            fragmentation: fragmentation.map(FragmentationState::new),
            out: Vec::new(),
        }
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
    pub(crate) fn fragmentation_mut(&mut self) -> Option<&mut FragmentationState> {
        self.fragmentation.as_mut()
    }

    /// Returns `true` when fragmentation is enabled.
    #[cfg(test)]
    pub(crate) fn has_fragmentation(&self) -> bool { self.fragmentation.is_some() }

    fn push_frame(&mut self, envelope: Envelope) {
        self.out.push(envelope);
        crate::metrics::inc_frames(crate::metrics::Direction::Outbound);
    }
}

/// Serialize an [`Envelope`] and write it through the codec to the framed
/// stream.
///
/// # Errors
///
/// Returns an [`io::Error`] if serialization or sending fails.
pub(super) async fn send_envelope<S, W, F>(
    serializer: &S,
    codec: &F,
    framed: &mut Framed<W, ConnectionCodec<F>>,
    envelope: &Envelope,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let bytes = serializer.serialize(envelope).map_err(|e| {
        let id = envelope.id;
        let correlation_id = envelope.correlation_id;
        warn!(
            "failed to serialize outbound envelope: id={id}, correlation_id={correlation_id:?}, \
             error={e:?}"
        );
        crate::metrics::inc_handler_errors();
        io::Error::other(e)
    })?;
    let frame = codec.wrap_payload(Bytes::from(bytes));
    framed.send(frame).await.map_err(|e| {
        let id = envelope.id;
        let correlation_id = envelope.correlation_id;
        warn!(
            "failed to send outbound frame: id={id}, correlation_id={correlation_id:?}, \
             error={e:?}"
        );
        crate::metrics::inc_handler_errors();
        io::Error::other(e)
    })
}

/// Flush a batch of pipeline-produced [`Envelope`] values through the codec
/// to the framed stream.
///
/// Each envelope is serialized, wrapped, and written individually. On the
/// first I/O failure the remaining envelopes are discarded and the error is
/// returned.
///
/// # Errors
///
/// Returns an [`io::Error`] if any envelope fails to serialize or send.
pub(super) async fn flush_pipeline_output<S, W, F>(
    serializer: &S,
    codec: &F,
    framed: &mut Framed<W, ConnectionCodec<F>>,
    envelopes: &mut Vec<Envelope>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    W: AsyncRead + AsyncWrite + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    for envelope in envelopes.drain(..) {
        send_envelope(serializer, codec, framed, &envelope).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    fn pipeline() -> FramePipeline {
        let config = None;
        FramePipeline::new(config)
    }

    #[rstest]
    fn process_single_envelope_emits_one_frame(mut pipeline: FramePipeline) {
        let env = Envelope::new(1, Some(42), vec![1, 2, 3]);
        pipeline
            .process(env)
            .expect("processing should succeed without fragmentation");
        let mut output = pipeline.drain_output();
        assert_eq!(output.len(), 1);
        let first = output
            .pop()
            .expect("pipeline should emit exactly one envelope");
        assert_eq!(first.id, 1);
        assert_eq!(first.correlation_id, Some(42));
        assert_eq!(first.payload, vec![1, 2, 3]);
    }

    #[rstest]
    fn drain_clears_buffer(mut pipeline: FramePipeline) {
        pipeline
            .process(Envelope::new(1, None, vec![]))
            .expect("processing should succeed without fragmentation");
        let first = pipeline.drain_output();
        assert_eq!(first.len(), 1);

        let second = pipeline.drain_output();
        assert!(second.is_empty());
    }

    #[rstest]
    fn pipeline_without_fragmentation(pipeline: FramePipeline) {
        assert!(!pipeline.has_fragmentation());
    }
}
