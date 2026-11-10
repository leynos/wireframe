//! Per-connection framing state for prepared inbound processing.
//!
//! Framing is built once per connection and then shared by the inbound reader
//! and the actor-driven response writer. Keeping the construction here leaves
//! the frame-processing loop focused on dispatch.

use futures::{
    StreamExt,
    stream::{SplitSink, SplitStream},
};
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use super::super::combined_codec::{CombinedCodec, ConnectionCodec};
use crate::codec::{FrameCodec, MAX_FRAME_LENGTH, clamp_frame_length};

/// Per-connection framing state shared by the inbound reader and response
/// writer.
pub(super) struct ConnectionFraming<W, F>
where
    F: FrameCodec,
{
    /// Codec clone that carries per-connection framing state.
    pub(super) codec: F,
    /// Framed stream built from the codec's decoder and encoder.
    pub(super) framed: Framed<W, CombinedCodec<F::Decoder, F::Encoder>>,
}

/// Write half of a framed connection transport.
pub(super) type TransportSink<W, F> =
    SplitSink<Framed<W, ConnectionCodec<F>>, <F as FrameCodec>::Frame>;

/// Read half of a framed connection transport.
pub(super) type TransportStream<W, F> = SplitStream<Framed<W, ConnectionCodec<F>>>;

impl<W, F> ConnectionFraming<W, F>
where
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Split the transport into its write sink, read stream, and codec.
    pub(super) fn split(self) -> (TransportSink<W, F>, TransportStream<W, F>, F) {
        let Self { codec, framed } = self;
        let (sink, stream) = framed.split();
        (sink, stream, codec)
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
    let combined = CombinedCodec::new(connection_codec.decoder(), connection_codec.encoder());
    let mut framed = Framed::new(stream, combined);
    let requested_frame_length = connection_codec.max_frame_length();
    let max_frame_length = clamp_frame_length(requested_frame_length);
    if requested_frame_length > MAX_FRAME_LENGTH {
        warn!(
            "codec max frame length exceeds guardrail; clamping to {MAX_FRAME_LENGTH} bytes \
             (requested={requested_frame_length})"
        );
    }
    framed.read_buffer_mut().reserve(max_frame_length);
    ConnectionFraming {
        codec: connection_codec,
        framed,
    }
}
