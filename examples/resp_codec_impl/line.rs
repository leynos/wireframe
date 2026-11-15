//! Bounded CRLF line scanning for the RESP parser.
//!
//! The scan never consumes partial input: it reports `Ok(None)` when the line
//! may still complete, and it enforces the line-length bound on every byte it
//! examines.

use std::io;

use bytes::BytesMut;

/// Find a bounded CRLF line without consuming partial input.
pub(super) fn parse_line(
    buf: &BytesMut,
    start: usize,
    max_len: usize,
) -> Result<Option<(&[u8], usize)>, io::Error> {
    let mut index = start;
    while buf.get(index).is_some() {
        match scan_byte(buf, start, index, max_len)? {
            LineScan::Found(line, next) => return Ok(Some((line, next))),
            LineScan::Incomplete => return Ok(None),
            LineScan::Continue => {}
        }
        index = index
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "line too long"))?;
    }
    Ok(None)
}

/// What examining one byte of a bounded line scan decided.
enum LineScan<'a> {
    /// Keep scanning at the next byte.
    Continue,
    /// A `\r` was the last buffered byte, so the line may still complete.
    Incomplete,
    /// The line, and the index just past its CRLF.
    Found(&'a [u8], usize),
}

/// Examine the byte at `index`, enforcing the line-length bound first.
fn scan_byte(
    buf: &BytesMut,
    start: usize,
    index: usize,
    max_len: usize,
) -> Result<LineScan<'_>, io::Error> {
    if index.saturating_sub(start) > max_len {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    if buf.get(index).copied() != Some(b'\r') {
        return Ok(LineScan::Continue);
    }
    match buf.get(index + 1).copied() {
        None => Ok(LineScan::Incomplete),
        Some(b'\n') => {
            let line = buf
                .get(start..index)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "line bounds"))?;
            Ok(LineScan::Found(line, index + 2))
        }
        Some(_) => Ok(LineScan::Continue),
    }
}
