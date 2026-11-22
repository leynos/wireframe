//! Framing primitives shared by the bundled example codecs.
//!
//! Both worked examples use a fixed-size header that declares the frame
//! length. The helpers here keep their decode paths identical without copying
//! one codec's body into the other, and they keep every truncation structured
//! so the connection can hand it to the protocol's `on_eof` hook.

use std::io;

use bytes::BytesMut;

use crate::codec::{CodecError, EofError};

/// The frame extent a complete header declares.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameExtent {
    /// Total frame length, header included.
    frame_len: usize,
    /// Declared payload length.
    payload_len: usize,
}

impl FrameExtent {
    /// Extent declared by a header of `header_len` bytes before `payload_len`.
    pub(super) const fn new(header_len: usize, payload_len: usize) -> Self {
        Self {
            frame_len: header_len + payload_len,
            payload_len,
        }
    }
}

/// A codec whose fixed-size header declares the frame length.
pub(super) trait FrameLayout: Sized {
    /// Header fields needed to build a frame.
    type Header;

    /// Size of the header in bytes.
    const HEADER_LEN: usize;

    /// Parse and validate a complete header.
    fn parse_header(&self, src: &[u8]) -> io::Result<Self::Header>;

    /// Extent the parsed header declares.
    fn extent(header: &Self::Header) -> FrameExtent;

    /// Buffered header and its extent, or `None` while the header is short.
    fn buffered_header(&self, src: &[u8]) -> io::Result<Option<(Self::Header, FrameExtent)>> {
        if src.len() < Self::HEADER_LEN {
            return Ok(None);
        }
        let header = self.parse_header(src)?;
        let extent = Self::extent(&header);
        Ok(Some((header, extent)))
    }

    /// Declared extent of the buffered frame, or `None` while the header is
    /// still incomplete.
    fn buffered_extent(&self, src: &[u8]) -> io::Result<Option<FrameExtent>> {
        Ok(self.buffered_header(src)?.map(|(_, extent)| extent))
    }
}

/// Decode one buffered frame, if the whole of it has arrived.
///
/// Nothing is consumed until a complete frame is present, so a partial frame
/// stays available for `decode_eof` to classify.
pub(super) fn decode_one<C, T>(
    codec: &C,
    src: &mut BytesMut,
    build: impl FnOnce(&mut BytesMut, C::Header) -> T,
) -> io::Result<Option<T>>
where
    C: FrameLayout,
{
    let Some((header, extent)) = codec.buffered_header(src)? else {
        return Ok(None);
    };
    if src.len() < extent.frame_len {
        return Ok(None);
    }
    Ok(Some(build(src, header)))
}

/// Finish a decode once the peer has closed.
///
/// A complete frame is handed to `decode`; an incomplete one reports a
/// structured end-of-file condition and leaves the buffer untouched, so the
/// connection can pass every received byte to `on_eof`.
pub(super) fn finish_decoding<C, T>(
    src: &mut BytesMut,
    declared: Option<FrameExtent>,
    decode: impl FnOnce(&mut BytesMut) -> io::Result<Option<T>>,
) -> io::Result<Option<T>>
where
    C: FrameLayout,
{
    if src.is_empty() {
        return Ok(None);
    }
    match declared {
        Some(extent) if src.len() >= extent.frame_len => decode(src),
        other => Err(truncation::<C>(src.len(), other)),
    }
}

/// Build the structured error for a frame the peer abandoned.
fn truncation<C: FrameLayout>(buffer_len: usize, declared: Option<FrameExtent>) -> io::Error {
    let condition = declared.map_or(
        EofError::MidHeader {
            bytes_received: buffer_len,
            header_size: C::HEADER_LEN,
        },
        |extent| EofError::MidFrame {
            bytes_received: buffer_len - C::HEADER_LEN,
            expected: extent.payload_len,
        },
    );
    CodecError::Eof(condition).into()
}
