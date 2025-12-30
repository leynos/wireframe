//! Combined codec adapter for `Framed` streams.

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::codec::FrameCodec;

pub(super) struct CombinedCodec<D, E> {
    decoder: D,
    encoder: E,
}

impl<D, E> CombinedCodec<D, E> {
    pub(super) fn new(decoder: D, encoder: E) -> Self { Self { decoder, encoder } }
}

pub(super) type ConnectionCodec<F> =
    CombinedCodec<<F as FrameCodec>::Decoder, <F as FrameCodec>::Encoder>;

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
