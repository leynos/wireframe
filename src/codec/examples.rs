//! Shared example codecs for tests and feature-gated examples.
//!
//! These implementations are intentionally minimal and meant to illustrate
//! custom framing. They are compiled for tests and for the `examples` feature
//! to avoid duplicating codec logic across example binaries.

use std::io;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use super::{CodecError, EofError, FrameCodec};

/// Bytes in a Hotline frame header.
const HOTLINE_HEADER_LEN: usize = 20;
/// Bytes in a `MySQL` frame header.
const MYSQL_HEADER_LEN: usize = 4;

/// A Hotline frame with a 20-byte big-endian header and transaction identifier.
#[derive(Clone, Debug)]
pub struct HotlineFrame {
    /// Transaction identifier from the Hotline frame header.
    pub transaction_id: u32,
    /// Frame payload following the Hotline header.
    pub payload: Bytes,
}

/// Codec for Hotline frames with 20-byte big-endian headers.
#[derive(Clone, Debug)]
pub struct HotlineFrameCodec {
    max_frame_length: usize,
}

impl HotlineFrameCodec {
    /// Construct a Hotline codec with a maximum frame length.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::codec::{FrameCodec, examples::HotlineFrameCodec};
    ///
    /// let codec = HotlineFrameCodec::new(1024);
    ///
    /// assert_eq!(codec.max_frame_length(), 1024);
    /// ```
    #[must_use]
    pub const fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

/// Fields a Hotline header supplies once the whole header is buffered.
#[derive(Clone, Copy, Debug)]
struct HotlineHeader {
    /// Declared payload length.
    payload_len: usize,
    /// Declared length of the header and payload together.
    frame_len: usize,
    /// Transaction identifier from the header.
    transaction_id: u32,
}

/// Adapter that encodes and decodes Hotline frames.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct HotlineAdapter {
    max_frame_length: usize,
}

impl HotlineAdapter {
    const fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }

    /// Parse and validate a complete Hotline header.
    fn parse_header(&self, src: &[u8]) -> Result<HotlineHeader, io::Error> {
        let mut header = src;
        let payload_len = header.get_u32() as usize;
        let frame_len = header.get_u32() as usize;
        let transaction_id = header.get_u32();
        if payload_len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "payload too large",
            ));
        }
        if frame_len != payload_len + HOTLINE_HEADER_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid total size",
            ));
        }
        Ok(HotlineHeader {
            payload_len,
            frame_len,
            transaction_id,
        })
    }
}

impl Decoder for HotlineAdapter {
    type Item = HotlineFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < HOTLINE_HEADER_LEN {
            return Ok(None);
        }

        let header = self.parse_header(src.as_ref())?;
        if src.len() < header.frame_len {
            return Ok(None);
        }

        let mut frame_bytes = src.split_to(header.frame_len);
        frame_bytes.advance(HOTLINE_HEADER_LEN);
        let payload = frame_bytes.freeze();

        Ok(Some(HotlineFrame {
            transaction_id: header.transaction_id,
            payload,
        }))
    }

    /// Classify an incomplete buffer now that the peer has closed.
    ///
    /// A malformed header still reports its framing error rather than a
    /// truncation, and the buffer is never consumed, so the connection can pass
    /// every received byte to the `on_eof` hook.
    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }
        if src.len() < HOTLINE_HEADER_LEN {
            return Err(CodecError::Eof(EofError::MidHeader {
                bytes_received: src.len(),
                header_size: HOTLINE_HEADER_LEN,
            })
            .into());
        }
        let header = self.parse_header(src.as_ref())?;
        if src.len() >= header.frame_len {
            return self.decode(src);
        }
        Err(CodecError::Eof(EofError::MidFrame {
            bytes_received: src.len() - HOTLINE_HEADER_LEN,
            expected: header.payload_len,
        })
        .into())
    }
}

impl Encoder<HotlineFrame> for HotlineAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: HotlineFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload_size = item.payload.len();
        if payload_size > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }
        let frame_size = payload_size + HOTLINE_HEADER_LEN;
        let wire_payload_size = u32::try_from(payload_size)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
        let wire_frame_size = u32::try_from(frame_size)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
        dst.reserve(wire_frame_size as usize);
        dst.put_u32(wire_payload_size);
        dst.put_u32(wire_frame_size);
        dst.put_u32(item.transaction_id);
        dst.put_bytes(0, 8);
        // The payload is copied into the transport buffer; record it so the
        // copied-byte baseline is comparable across codecs.
        crate::benchmark_counters::record_framed(payload_size);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

impl FrameCodec for HotlineFrameCodec {
    type Frame = HotlineFrame;
    type Decoder = HotlineAdapter;
    type Encoder = HotlineAdapter;

    fn decoder(&self) -> Self::Decoder { HotlineAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { HotlineAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { &frame.payload }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.payload.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        HotlineFrame {
            transaction_id: 0,
            payload,
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.transaction_id)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

/// A `MySQL` frame with a 3-byte little-endian length and sequence number.
#[derive(Clone, Debug)]
pub struct MysqlFrame {
    /// Sequence number from the `MySQL` frame header.
    pub sequence_id: u8,
    /// Frame payload following the `MySQL` header.
    pub payload: Bytes,
}

/// Codec for `MySQL` frames with 3-byte little-endian length headers.
#[derive(Clone, Debug)]
pub struct MysqlFrameCodec {
    max_frame_length: usize,
}

impl MysqlFrameCodec {
    /// Construct a `MySQL` codec with a maximum frame length.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::codec::{FrameCodec, examples::MysqlFrameCodec};
    ///
    /// let codec = MysqlFrameCodec::new(1024);
    ///
    /// assert_eq!(codec.max_frame_length(), 1024);
    /// ```
    #[must_use]
    pub const fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

/// Adapter that encodes and decodes `MySQL` frames.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct MysqlAdapter {
    max_frame_length: usize,
}

/// Fields a `MySQL` header supplies once the whole header is buffered.
#[derive(Clone, Copy, Debug)]
struct MysqlHeader {
    /// Declared payload length.
    payload_len: usize,
    /// Sequence number from the header.
    sequence_id: u8,
}

impl MysqlAdapter {
    const fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }

    /// Parse and validate a complete `MySQL` header.
    fn parse_header(&self, src: &[u8]) -> Result<MysqlHeader, io::Error> {
        let mut header = src;
        let payload_len = (header.get_u8() as usize)
            | ((header.get_u8() as usize) << 8)
            | ((header.get_u8() as usize) << 16);
        let sequence_id = header.get_u8();
        if payload_len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "payload too large",
            ));
        }
        Ok(MysqlHeader {
            payload_len,
            sequence_id,
        })
    }
}

impl Decoder for MysqlAdapter {
    type Item = MysqlFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < MYSQL_HEADER_LEN {
            return Ok(None);
        }

        let header = self.parse_header(src.as_ref())?;
        let frame_len = MYSQL_HEADER_LEN + header.payload_len;
        if src.len() < frame_len {
            return Ok(None);
        }

        let mut frame_bytes = src.split_to(frame_len);
        frame_bytes.advance(MYSQL_HEADER_LEN);
        let payload = frame_bytes.freeze();

        Ok(Some(MysqlFrame {
            sequence_id: header.sequence_id,
            payload,
        }))
    }

    /// Classify an incomplete buffer now that the peer has closed.
    ///
    /// A malformed header still reports its framing error rather than a
    /// truncation, and the buffer is never consumed, so the connection can pass
    /// every received byte to the `on_eof` hook.
    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }
        if src.len() < MYSQL_HEADER_LEN {
            return Err(CodecError::Eof(EofError::MidHeader {
                bytes_received: src.len(),
                header_size: MYSQL_HEADER_LEN,
            })
            .into());
        }
        let header = self.parse_header(src.as_ref())?;
        if src.len() >= MYSQL_HEADER_LEN + header.payload_len {
            return self.decode(src);
        }
        Err(CodecError::Eof(EofError::MidFrame {
            bytes_received: src.len() - MYSQL_HEADER_LEN,
            expected: header.payload_len,
        })
        .into())
    }
}

impl Encoder<MysqlFrame> for MysqlAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: MysqlFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload_len = item.payload.len();
        if payload_len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }
        if payload_len > 0xff_ff_ff {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too long",
            ));
        }

        let payload_len_u32 = u32::try_from(payload_len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too long"))?;
        let len_lo = (payload_len_u32 & 0xff) as u8;
        let len_mid = ((payload_len_u32 >> 8) & 0xff) as u8;
        let len_hi = ((payload_len_u32 >> 16) & 0xff) as u8;

        dst.reserve(MYSQL_HEADER_LEN + payload_len);
        dst.put_u8(len_lo);
        dst.put_u8(len_mid);
        dst.put_u8(len_hi);
        dst.put_u8(item.sequence_id);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

impl FrameCodec for MysqlFrameCodec {
    type Frame = MysqlFrame;
    type Decoder = MysqlAdapter;
    type Encoder = MysqlAdapter;

    fn decoder(&self) -> Self::Decoder { MysqlAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { MysqlAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { &frame.payload }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.payload.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        MysqlFrame {
            sequence_id: 0,
            payload,
        }
    }

    // MySQL sequence_id is a packet counter, not a true correlation ID.
    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.sequence_id)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}
