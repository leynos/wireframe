//! Adapts a frame decoder so decode failures become in-band stream items.
//!
//! A `Framed` stream treats a decoder error as terminal: the error is yielded
//! once and the caller cannot resume reading. Recovery policies need to observe
//! the failure, decide whether the connection can continue, and keep the framed
//! transport alive for a later frame. This adapter converts each decoder `Err`
//! into a [`DecodedItem::Failure`] value so the connection loop can evaluate a
//! policy. Errors raised by the socket read itself still come from `Framed` and
//! remain terminal.

use std::io;

use bytes::BytesMut;
use tokio_util::codec::Decoder;

/// The decoder entry point that produced a failure.
///
/// `decode_eof` failures are terminal by definition: the peer has stopped
/// sending, so no later frame can justify continuing the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DecodeOrigin {
    /// The failure came from [`Decoder::decode`].
    Decode,
    /// The failure came from [`Decoder::decode_eof`].
    DecodeEof,
}

/// A decoder failure captured for recovery evaluation.
///
/// The adapter owns the original [`io::Error`] so the connection loop can
/// return it unchanged when the applied policy disconnects. Rebuilding the
/// error from its display text would lose its kind and source chain.
#[derive(Debug)]
pub(super) struct DecodeFailure {
    /// Original error returned by the wrapped decoder.
    error: io::Error,
    /// Entry point that produced the error.
    origin: DecodeOrigin,
    /// Bytes the failing call removed from the read buffer.
    consumed: usize,
}

impl DecodeFailure {
    /// Bundle an error with the call site and consumption it was observed at.
    pub(super) const fn new(error: io::Error, origin: DecodeOrigin, consumed: usize) -> Self {
        Self {
            error,
            origin,
            consumed,
        }
    }

    /// Split the failure into its original error, origin, and consumption.
    pub(super) fn into_parts(self) -> (io::Error, DecodeOrigin, usize) {
        (self.error, self.origin, self.consumed)
    }

    /// Entry point that produced this failure.
    pub(super) const fn origin(&self) -> DecodeOrigin { self.origin }
}

/// One decoder result: either a real frame or a captured failure.
pub(super) enum DecodedItem<Frame> {
    /// The decoder produced a frame that can be handled normally.
    Frame(Frame),
    /// The decoder failed; the connection loop decides what happens next.
    Failure(DecodeFailure),
}

impl<Frame> DecodedItem<Frame> {
    /// Return the decoded frame, failing the calling test on a decode error.
    ///
    /// Tests that drive a duplex transport with well-formed client traffic use
    /// this to keep the assertion focused on what reached the wire.
    #[cfg(test)]
    pub(super) fn expect_frame(self) -> Frame {
        match self {
            Self::Frame(frame) => frame,
            Self::Failure(failure) => panic!("decoder failed unexpectedly: {failure:?}"),
        }
    }
}

/// Wraps a decoder so its errors become [`DecodedItem::Failure`] items.
pub(super) struct RecoveryDecoder<D> {
    /// Decoder owned by the framed transport for this connection.
    inner: D,
}

impl<D> RecoveryDecoder<D> {
    /// Adopt a decoder without changing its buffering or state semantics.
    pub(super) const fn new(inner: D) -> Self { Self { inner } }
}

impl<D> Decoder for RecoveryDecoder<D>
where
    D: Decoder<Error = io::Error>,
{
    type Item = DecodedItem<D::Item>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(capture(
            &mut self.inner,
            src,
            DecodeOrigin::Decode,
            D::decode,
        ))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(capture(
            &mut self.inner,
            src,
            DecodeOrigin::DecodeEof,
            D::decode_eof,
        ))
    }
}

/// Run one decoder call and record any failure without propagating it.
///
/// Consumption is measured from the buffer length so a decoder that leaves the
/// malformed bytes in place is distinguishable from one that advanced past
/// them. The connection loop uses that distinction to avoid retrying the same
/// bytes forever.
fn capture<D>(
    decoder: &mut D,
    src: &mut BytesMut,
    origin: DecodeOrigin,
    decode: impl FnOnce(&mut D, &mut BytesMut) -> io::Result<Option<D::Item>>,
) -> Option<DecodedItem<D::Item>>
where
    D: Decoder<Error = io::Error>,
{
    let before = src.len();
    match decode(decoder, src) {
        Ok(Some(item)) => Some(DecodedItem::Frame(item)),
        Ok(None) => None,
        Err(error) => Some(DecodedItem::Failure(DecodeFailure::new(
            error,
            origin,
            before.saturating_sub(src.len()),
        ))),
    }
}

#[cfg(test)]
#[path = "recovery_codec_tests.rs"]
mod tests;
