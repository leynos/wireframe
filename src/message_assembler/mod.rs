//! Protocol-level message assembly hooks.
//!
//! A `MessageAssembler` parses protocol-specific frame headers to distinguish
//! "first" frames from "continuation" frames. The hook operates above
//! transport fragmentation and feeds the streaming request pipeline once the
//! connection actor integrates it (see ADR 0002).
//!
//! ## Message key multiplexing (8.2.3)
//!
//! The [`MessageAssemblyState`] type manages multiple concurrent assemblies
//! keyed by [`MessageKey`], enabling interleaved frame streams from different
//! logical messages on the same connection.
//!
//! ## Continuity validation (8.2.4)
//!
//! The [`MessageSeries`] type validates frame ordering when protocols supply
//! sequence numbers via [`ContinuationFrameHeader::sequence`]. It detects:
//!
//! - Out-of-order frames (sequence gaps)
//! - Duplicate frames (already-processed sequences)
//! - Frames arriving after the series is complete

pub mod error;
mod header;
pub mod series;
pub mod state;

use std::io;

pub use error::{MessageAssemblyError, MessageSeriesError, MessageSeriesStatus};
pub use header::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameHeader,
    FrameSequence,
    MessageKey,
    ParsedFrameHeader,
};
pub use series::MessageSeries;
pub use state::{AssembledMessage, MessageAssemblyState};

/// Hook trait for protocol-specific multi-frame request parsing.
///
/// Implementations should parse only the per-frame header and return the
/// parsed header plus the number of bytes consumed. The remaining bytes are
/// treated as the frame's body chunk.
///
/// # Examples
///
/// ```rust,no_run
/// use bytes::Buf;
/// use wireframe::message_assembler::{
///     FirstFrameHeader,
///     FrameHeader,
///     MessageAssembler,
///     MessageKey,
///     ParsedFrameHeader,
/// };
///
/// struct DemoAssembler;
///
/// impl MessageAssembler for DemoAssembler {
///     fn parse_frame_header(&self, payload: &[u8]) -> Result<ParsedFrameHeader, std::io::Error> {
///         let mut buf = payload;
///         if buf.remaining() < 9 {
///             return Err(std::io::Error::new(
///                 std::io::ErrorKind::InvalidData,
///                 "header too short",
///             ));
///         }
///
///         let tag = buf.get_u8();
///         let key = MessageKey::from(buf.get_u64());
///         let header = match tag {
///             0x01 => FrameHeader::First(FirstFrameHeader {
///                 message_key: key,
///                 metadata_len: 0,
///                 body_len: buf.remaining(),
///                 total_body_len: None,
///                 is_last: true,
///             }),
///             _ => {
///                 return Err(std::io::Error::new(
///                     std::io::ErrorKind::InvalidData,
///                     "unknown header tag",
///                 ));
///             }
///         };
///
///         let header_len = payload.len() - buf.remaining();
///         Ok(ParsedFrameHeader::new(header, header_len))
///     }
/// }
/// ```
pub trait MessageAssembler: Send + Sync + 'static {
    /// Parse a protocol header from the provided payload bytes.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` when the header is malformed or incomplete.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::message_assembler::{MessageAssembler, ParsedFrameHeader};
    ///
    /// struct Demo;
    ///
    /// impl MessageAssembler for Demo {
    ///     fn parse_frame_header(&self, _payload: &[u8]) -> Result<ParsedFrameHeader, std::io::Error> {
    ///         Err(std::io::Error::new(
    ///             std::io::ErrorKind::InvalidData,
    ///             "header not implemented",
    ///         ))
    ///     }
    /// }
    /// ```
    fn parse_frame_header(&self, payload: &[u8]) -> Result<ParsedFrameHeader, io::Error>;
}

#[cfg(test)]
mod tests;
