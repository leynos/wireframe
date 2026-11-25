//! Length-prefixed codec and frame builders used by the recovery cases.
//!
//! The codec is deliberately small so each case can state the failure it
//! provokes: a marker payload is rejected after its frame is consumed, an
//! over-long length prefix is a framing error, and a peer that closes
//! mid-frame reaches `decode_eof` with a partial frame still buffered.

use super::*;

/// Codec that frames payloads with a big-endian length prefix.
#[derive(Clone, Debug)]
pub(super) struct TestCodec {
    /// Largest payload accepted.
    pub(super) max: usize,
}

impl FrameCodec for TestCodec {
    type Frame = Bytes;
    type Decoder = TestDecoder;
    type Encoder = TestEncoder;

    fn decoder(&self) -> Self::Decoder { TestDecoder { max: self.max } }

    fn encoder(&self) -> Self::Encoder { TestEncoder { max: self.max } }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.as_ref() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame { payload }

    fn max_frame_length(&self) -> usize { self.max }
}

/// Decoder that rejects a marker payload after consuming its frame.
#[derive(Debug)]
pub(super) struct TestDecoder {
    /// Largest payload accepted.
    max: usize,
}

impl Decoder for TestDecoder {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }
        // Read the length through `Buf` so the prefix is not indexed directly.
        let mut header: &[u8] = src.as_ref();
        let length = header.get_u32() as usize;
        if length > self.max {
            return Err(CodecError::Framing(FramingError::OversizedFrame {
                size: length,
                max: self.max,
            })
            .into());
        }
        if src.len() < 4 + length {
            return Ok(None);
        }
        // Consuming the frame first keeps the connection at a frame boundary,
        // which is what lets a recovery policy continue after the error.
        let mut frame = src.split_to(4 + length);
        let payload = frame.split_off(4).freeze();
        if payload.first() == Some(&MALFORMED) {
            return Err(CodecError::Protocol(ProtocolError::UnknownMessageType {
                type_id: u32::from(MALFORMED),
            })
            .into());
        }
        Ok(Some(payload))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }
        // The peer closed while a frame was still arriving, so the leftover
        // bytes can never become one. Consuming them keeps the decoder at a
        // boundary and makes the failure report what was lost.
        let (bytes_received, expected) = eof_progress(src);
        src.clear();
        Err(CodecError::Eof(EofError::MidFrame {
            bytes_received,
            expected,
        })
        .into())
    }
}

/// Split a truncated buffer into the payload bytes received and those declared.
fn eof_progress(src: &BytesMut) -> (usize, usize) {
    if src.len() < 4 {
        return (src.len(), 4);
    }
    let mut header: &[u8] = src.as_ref();
    let declared = usize::try_from(header.get_u32()).unwrap_or(usize::MAX);
    (src.len().saturating_sub(4), declared)
}

/// Encoder mirroring the decoder's length-prefixed framing.
#[derive(Debug)]
pub(super) struct TestEncoder {
    /// Largest payload accepted.
    max: usize,
}

impl Encoder<Bytes> for TestEncoder {
    type Error = io::Error;

    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.len() > self.max {
            return Err(CodecError::Framing(FramingError::OversizedFrame {
                size: item.len(),
                max: self.max,
            })
            .into());
        }
        let length = u32::try_from(item.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "payload length exceeds u32")
        })?;
        dst.put_u32(length);
        dst.extend_from_slice(&item);
        Ok(())
    }
}

/// Length-prefixed frame carrying the given bytes.
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = BytesMut::with_capacity(4 + payload.len());
    bytes.put_u32(u32::try_from(payload.len()).unwrap_or(u32::MAX));
    bytes.extend_from_slice(payload);
    bytes.to_vec()
}

/// Frame carrying a routed envelope with `payload`.
pub(super) fn envelope_frame(payload: &[u8]) -> TestResult<Vec<u8>> {
    let envelope = Envelope::new(MESSAGE_ID, Some(7), payload.to_vec());
    let encoded = BincodeSerializer.serialize(&envelope)?;
    Ok(frame(&encoded))
}

/// Frame whose payload the decoder rejects after consuming it.
pub(super) fn malformed_frame() -> Vec<u8> {
    let mut payload = vec![MALFORMED];
    payload.extend_from_slice(b"malformed");
    frame(&payload)
}

/// A frame header declaring an eight-byte payload that carries one byte.
///
/// The peer closes after sending it, so the decoder reaches end of input with
/// a partial frame instead of a complete one.
pub(super) fn truncated_frame() -> Vec<u8> {
    let mut bytes = BytesMut::with_capacity(5);
    bytes.put_u32(8);
    bytes.extend_from_slice(b"x");
    bytes.to_vec()
}

/// A length prefix declaring one byte more than `max`, with no payload.
pub(super) fn oversized_frame(max: usize) -> Vec<u8> {
    let mut bytes = BytesMut::with_capacity(4);
    bytes.put_u32(u32::try_from(max.saturating_add(1)).unwrap_or(u32::MAX));
    bytes.to_vec()
}
