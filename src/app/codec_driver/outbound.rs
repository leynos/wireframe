//! Outbound frame transmission for the codec-aware connection driver.
//!
//! These helpers serialize handler responses, wrap them with the connection's
//! [`FrameCodec`], run the protocol `before_send` hook, and write each frame to
//! the transport sink. They also write protocol-supplied frames such as stream
//! terminators, which are complete codec frames and must reach the wire
//! unchanged.

use futures::{Sink, SinkExt};
use log::warn;
use tokio::io;

use super::FramePipeline;
use crate::{
    app::{Envelope, outbound_encoding::encode_message_frame},
    codec::{CodecError, FrameCodec},
    connection::FrameSink,
    message::EncodeWith,
    metrics::{AppResponseSendOutcome, Direction, inc_app_response_send, inc_frames},
    serializer::Serializer,
};

/// Outbound transport parameters shared by the frame-sending helpers.
///
/// The serializer, codec, and framed stream always travel together, so
/// grouping them keeps the send helpers within the argument limits the
/// project lints enforce.
pub(crate) struct OutboundTransport<'a, S, K, F>
where
    F: FrameCodec,
{
    /// Serializer converting an envelope into payload bytes.
    pub(crate) serializer: &'a S,
    /// Frame codec wrapping payloads into wire frames.
    pub(crate) codec: &'a F,
    /// Sink receiving encoded frames.
    pub(crate) sink: &'a mut K,
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
pub(crate) async fn send_envelope<S, K, F>(
    transport: &mut OutboundTransport<'_, S, K, F>,
    envelope: &Envelope,
    pipeline: &mut FramePipeline<F::Frame>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin,
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
    pipeline.before_send_frame(&mut encoded.frame);
    match transport.sink.send(encoded.frame).await {
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
pub(crate) async fn flush_pipeline_output<S, K, F>(
    transport: &mut OutboundTransport<'_, S, K, F>,
    pipeline: &mut FramePipeline<F::Frame>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    let mut envelopes = pipeline.drain_output();
    for envelope in envelopes.drain(..) {
        send_envelope(transport, &envelope, pipeline).await?;
    }
    Ok(())
}

/// Write a frame supplied by the protocol, unchanged, through the sink.
///
/// Terminators and other protocol-specific frames that arrive from
/// [`WireframeProtocol::stream_end_frame`] are complete codec frames. A custom
/// codec may encode metadata outside the payload — tags, sequencing state, or
/// padding — that neither [`FrameCodec::frame_payload`] nor
/// [`FrameCodec::wrap_payload`] preserves. Writing the frame the protocol
/// returned, rather than decoding it to an [`Envelope`] and wrapping it again,
/// keeps that metadata intact. The connection's `before_send` hook still runs
/// on the exact frame about to be written, matching application frames.
///
/// [`WireframeProtocol::stream_end_frame`]: crate::hooks::WireframeProtocol::stream_end_frame
///
/// # Errors
///
/// Returns an [`io::Error`] if the sink rejects the frame.
pub(crate) async fn send_native_frame<K, F>(
    sink: &mut K,
    pipeline: &mut FramePipeline<F::Frame>,
    mut frame: F::Frame,
) -> io::Result<()>
where
    K: Sink<F::Frame, Error = io::Error> + Unpin,
    F: FrameCodec,
{
    pipeline.before_send_frame(&mut frame);
    match sink.send(frame).await {
        Ok(()) => {
            inc_frames(Direction::Outbound);
            inc_app_response_send(AppResponseSendOutcome::Success);
            Ok(())
        }
        Err(error) => {
            let outcome = classify_send_error(&error);
            warn!(
                "failed to send protocol frame: stage=write, category={}, error={error:?}",
                outcome.as_str()
            );
            crate::metrics::inc_handler_errors();
            inc_app_response_send(outcome);
            Err(io::Error::other(error))
        }
    }
}

/// Sink that writes actor-produced envelopes through the codec pipeline.
///
/// The actor schedules responses; this sink is where they become bytes. Each
/// envelope is fragmented and counted by the [`FramePipeline`], serialized and
/// wrapped by the codec, passed through the protocol `before_send` hook, and
/// then written to the framed stream. Every physical frame is awaited, so a
/// bounded transport applies back-pressure to the actor before it polls the
/// next source.
pub(crate) struct ActorFrameSink<'a, S, K, F>
where
    F: FrameCodec,
{
    /// Serializer converting an envelope into payload bytes.
    serializer: &'a S,
    /// Frame codec wrapping payloads into wire frames.
    codec: &'a F,
    /// Sink receiving encoded frames.
    sink: &'a mut K,
    /// Outbound pipeline applying fragmentation, hooks, and metrics.
    pipeline: &'a mut FramePipeline<F::Frame>,
}

impl<'a, S, K, F> ActorFrameSink<'a, S, K, F>
where
    F: FrameCodec,
{
    /// Borrow the transport and pipeline that this sink writes through.
    pub(crate) const fn new(
        serializer: &'a S,
        codec: &'a F,
        sink: &'a mut K,
        pipeline: &'a mut FramePipeline<F::Frame>,
    ) -> Self {
        Self {
            serializer,
            codec,
            sink,
            pipeline,
        }
    }
}

impl<S, K, F> FrameSink<Envelope> for ActorFrameSink<'_, S, K, F>
where
    S: Serializer + Send + Sync,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: EncodeWith<S>,
{
    async fn send_frame(&mut self, envelope: Envelope) -> io::Result<()> {
        self.pipeline.process(envelope)?;
        let mut transport = OutboundTransport {
            serializer: self.serializer,
            codec: self.codec,
            sink: &mut *self.sink,
        };
        // `flush_pipeline_output` drops any remaining fragments on the
        // first send failure, which is the required error behaviour.
        flush_pipeline_output(&mut transport, self.pipeline).await
    }
}
