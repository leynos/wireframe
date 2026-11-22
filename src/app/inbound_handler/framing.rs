//! Per-connection framing state for prepared inbound processing.
//!
//! Framing is built once per connection and then shared by the inbound reader
//! and the actor-driven response writer. The read and write halves are separate
//! [`FramedRead`] and [`FramedWrite`] values rather than two halves of one
//! `Framed`. That keeps the read half's buffer accessible: when a peer
//! disconnects mid-frame, the connection reports the protocol `on_eof` hook
//! with exactly the bytes buffered for the frame that never completed.

use std::pin::Pin;

use log::warn;
use tokio::io::{self, AsyncRead, AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::codec::{FramedRead, FramedWrite};

use super::super::codec_driver::FramePipeline;
use crate::codec::{EofError, FrameCodec, MAX_FRAME_LENGTH, clamp_frame_length, eof_condition};

/// Write half of a framed connection transport.
///
/// The box is pinned because `FramedWrite` is only `Unpin` when the codec's
/// encoder is, and the encoder type is not required to be `Unpin`. Boxing pins
/// the value without adding that bound to the public framing API.
pub(super) type TransportSink<W, F> =
    Pin<Box<FramedWrite<WriteHalf<W>, <F as FrameCodec>::Encoder>>>;

/// Read half of a framed connection transport.
///
/// Pinned for the same reason as [`TransportSink`]. `FramedRead` keeps decoded
/// leftovers in its own buffer, which
/// [`FramedRead::read_buffer`] exposes to the EOF hook.
pub(super) type TransportStream<W, F> =
    Pin<Box<FramedRead<ReadHalf<W>, <F as FrameCodec>::Decoder>>>;

/// The bytes a framed reader still holds for a frame the peer abandoned.
///
/// Naming the value keeps the reader's invariant where it is relied upon: an
/// incomplete frame is never consumed, so what the reader holds is exactly what
/// arrived for it. A bare `read_buffer` call leaves that reasoning implicit at
/// every use, and the end-of-file hook's input then depends on a reader
/// behaviour no type expresses.
#[derive(Clone, Copy, Debug)]
pub(super) struct TruncatedFrame<'a>(&'a [u8]);

impl<'a> TruncatedFrame<'a> {
    /// Every byte received for the frame that never completed.
    ///
    /// Built-in codecs leave the length prefix in place, so this includes it. A
    /// custom decoder that consumes bytes before reporting its end-of-file
    /// condition yields only the bytes it left behind.
    pub(super) const fn as_bytes(&self) -> &'a [u8] { self.0 }
}

/// Read what the framed reader still holds for an abandoned frame.
pub(super) fn truncated_frame<W, F>(
    reader: &FramedRead<ReadHalf<W>, F::Decoder>,
) -> TruncatedFrame<'_>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    TruncatedFrame(reader.read_buffer().as_ref())
}

/// Report a read error to the protocol's end-of-file hook.
///
/// Every error that ends the connection loop is passed here, whichever step
/// observed it, so a truncation seen while a handler or a push write is pending
/// is reported exactly once. Only a structured mid-header or mid-frame codec
/// error describes a truncation; transport failures, decode failures, and a
/// clean close at a frame boundary are reported by their own paths or not at
/// all. The partial data is every byte the framed reader still held for the
/// incomplete frame: the built-in codecs leave the length prefix in place, so
/// it is included, while a decoder that consumes bytes before reporting its
/// condition yields only what it left behind.
pub(super) fn report_truncation<W, F>(
    reader: &FramedRead<ReadHalf<W>, F::Decoder>,
    pipeline: &mut FramePipeline<F::Frame>,
    error: &io::Error,
) where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    let partial = truncated_frame::<W, F>(reader);
    match eof_condition(error) {
        Some(eof @ (EofError::MidHeader { .. } | EofError::MidFrame { .. })) => {
            tracing::debug!(
                condition = ?eof,
                partial_bytes = partial.as_bytes().len(),
                "reporting a truncated frame to the protocol"
            );
            pipeline.on_eof(&eof, partial.as_bytes());
        }
        // A clean close loses nothing, so there is no partial frame to report.
        Some(EofError::CleanClose) | None => {}
    }
}

/// Per-connection framing state shared by the inbound reader and response
/// writer.
pub(super) struct ConnectionFraming<W, F>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Codec clone that carries per-connection framing state.
    pub(super) codec: F,
    /// Writer built from the codec's encoder.
    pub(super) sink: TransportSink<W, F>,
    /// Reader built from the codec's decoder.
    pub(super) stream: TransportStream<W, F>,
}

impl<W, F> ConnectionFraming<W, F>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Split the transport into its write sink, read stream, and codec.
    pub(super) fn split(self) -> (TransportSink<W, F>, TransportStream<W, F>, F) {
        (self.sink, self.stream, self.codec)
    }
}

/// Build per-connection framing state and guard the requested frame length.
pub(super) fn prepare_connection_framing<W, F>(stream: W, codec: &F) -> ConnectionFraming<W, F>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    // Each connection needs isolated framing state: cloning resets the
    // counters `SeqFrameCodec` and `TaggedFrameCodec::wrap_payload` consume.
    let connection_codec = codec.clone();
    let requested_frame_length = connection_codec.max_frame_length();
    let max_frame_length = clamp_frame_length(requested_frame_length);
    if requested_frame_length > MAX_FRAME_LENGTH {
        warn!(
            "codec max frame length exceeds guardrail; clamping to {MAX_FRAME_LENGTH} bytes \
             (requested={requested_frame_length})"
        );
    }
    let (read_half, write_half) = tokio::io::split(stream);
    let framed_read =
        FramedRead::with_capacity(read_half, connection_codec.decoder(), max_frame_length);
    let sink = FramedWrite::new(write_half, connection_codec.encoder());
    ConnectionFraming {
        codec: connection_codec,
        sink: Box::pin(sink),
        stream: Box::pin(framed_read),
    }
}
