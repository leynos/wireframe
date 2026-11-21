//! Combined codec adapter for `Framed` streams.

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use super::recovery_codec::RecoveryDecoder;
use crate::codec::FrameCodec;

/// Adapter combining independent decoder and encoder halves for `Framed`.
pub(super) struct CombinedCodec<D, E> {
    /// Decoder that consumes bytes from the transport.
    decoder: D,
    /// Encoder that writes protocol frames to the transport.
    encoder: E,
}

impl<D, E> CombinedCodec<D, E> {
    /// Pair codec halves without changing either half's buffering semantics.
    pub(super) const fn new(decoder: D, encoder: E) -> Self { Self { decoder, encoder } }
}

/// Codec type selected from a [`FrameCodec`] implementation.
///
/// The decoder half is wrapped by [`RecoveryDecoder`], so the stream item is a
/// [`DecodedItem`](super::recovery_codec::DecodedItem) carrying either a real
/// frame or a recovery failure. The encoder half is unchanged, so the sink
/// still accepts `F::Frame`.
pub(super) type ConnectionCodec<F> =
    CombinedCodec<RecoveryDecoder<<F as FrameCodec>::Decoder>, <F as FrameCodec>::Encoder>;

/// Build the recovery-aware connection codec from a codec template.
///
/// Cloning and constructing happen per connection so stateful decoders and
/// encoders start from the template's initial state.
pub(super) fn connection_codec<F: FrameCodec>(codec: &F) -> ConnectionCodec<F> {
    CombinedCodec::new(RecoveryDecoder::new(codec.decoder()), codec.encoder())
}

impl<D, E> Decoder for CombinedCodec<D, E>
where
    D: Decoder,
{
    type Item = D::Item;
    type Error = D::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode(src)
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode_eof(src)
    }
}

impl<D, E, Item> Encoder<Item> for CombinedCodec<D, E>
where
    E: Encoder<Item>,
{
    type Error = E::Error;

    fn encode(&mut self, item: Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.encoder.encode(item, dst)
    }
}
