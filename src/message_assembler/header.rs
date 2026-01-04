//! Header model for protocol-level message assembly.
//!
//! These types describe per-frame metadata required to reassemble multi-frame
//! requests. Protocol crates populate them via `MessageAssembler`.

use std::fmt;

/// Identifies a logical message being assembled from multiple frames.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MessageKey(pub u64);

impl From<u64> for MessageKey {
    fn from(value: u64) -> Self { Self(value) }
}

impl From<MessageKey> for u64 {
    fn from(value: MessageKey) -> Self { value.0 }
}

impl fmt::Display for MessageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

/// Optional sequence number for continuation frames.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameSequence(pub u32);

impl From<u32> for FrameSequence {
    fn from(value: u32) -> Self { Self(value) }
}

impl From<FrameSequence> for u32 {
    fn from(value: FrameSequence) -> Self { value.0 }
}

impl fmt::Display for FrameSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

/// Parsed per-frame header information.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameHeader {
    /// Header describing the first frame in a multi-frame request.
    First(FirstFrameHeader),
    /// Header describing a continuation frame.
    Continuation(ContinuationFrameHeader),
}

/// Header metadata for the first frame in a message series.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FirstFrameHeader {
    /// Key used to correlate frames belonging to the same message.
    pub message_key: MessageKey,
    /// Protocol-specific metadata length in bytes.
    pub metadata_len: usize,
    /// Length of the body bytes in this frame.
    pub body_len: usize,
    /// Total expected body length across all frames, if declared by the protocol.
    pub total_body_len: Option<usize>,
    /// Whether this frame completes the message.
    pub is_last: bool,
}

/// Header metadata for continuation frames.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationFrameHeader {
    /// Key used to correlate frames belonging to the same message.
    pub message_key: MessageKey,
    /// Optional frame sequence index, when supplied by the protocol.
    pub sequence: Option<FrameSequence>,
    /// Length of the body bytes in this frame.
    pub body_len: usize,
    /// Whether this frame completes the message.
    pub is_last: bool,
}

/// Result of parsing a frame header from payload bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedFrameHeader {
    header: FrameHeader,
    header_len: usize,
}

impl ParsedFrameHeader {
    /// Create a parsed header with its byte length.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::message_assembler::{
    ///     FirstFrameHeader,
    ///     FrameHeader,
    ///     MessageKey,
    ///     ParsedFrameHeader,
    /// };
    ///
    /// let header = FrameHeader::First(FirstFrameHeader {
    ///     message_key: MessageKey(1),
    ///     metadata_len: 0,
    ///     body_len: 4,
    ///     total_body_len: None,
    ///     is_last: true,
    /// });
    ///
    /// let parsed = ParsedFrameHeader::new(header, 12);
    /// assert_eq!(parsed.header_len(), 12);
    /// ```
    #[must_use]
    pub const fn new(header: FrameHeader, header_len: usize) -> Self { Self { header, header_len } }

    /// Return the parsed header information.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::message_assembler::{
    ///     FirstFrameHeader,
    ///     FrameHeader,
    ///     MessageKey,
    ///     ParsedFrameHeader,
    /// };
    ///
    /// let header = FrameHeader::First(FirstFrameHeader {
    ///     message_key: MessageKey(7),
    ///     metadata_len: 0,
    ///     body_len: 1,
    ///     total_body_len: None,
    ///     is_last: true,
    /// });
    /// let parsed = ParsedFrameHeader::new(header, 4);
    /// assert!(matches!(parsed.header(), FrameHeader::First(_)));
    /// ```
    #[must_use]
    pub const fn header(&self) -> &FrameHeader { &self.header }

    /// Return the number of bytes consumed by the header.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::message_assembler::{
    ///     FirstFrameHeader,
    ///     FrameHeader,
    ///     MessageKey,
    ///     ParsedFrameHeader,
    /// };
    ///
    /// let header = FrameHeader::First(FirstFrameHeader {
    ///     message_key: MessageKey(3),
    ///     metadata_len: 0,
    ///     body_len: 0,
    ///     total_body_len: None,
    ///     is_last: true,
    /// });
    /// let parsed = ParsedFrameHeader::new(header, 12);
    /// assert_eq!(parsed.header_len(), 12);
    /// ```
    #[must_use]
    pub const fn header_len(&self) -> usize { self.header_len }

    /// Consume the parsed header and return the underlying header value.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::message_assembler::{
    ///     FirstFrameHeader,
    ///     FrameHeader,
    ///     MessageKey,
    ///     ParsedFrameHeader,
    /// };
    ///
    /// let header = FrameHeader::First(FirstFrameHeader {
    ///     message_key: MessageKey(1),
    ///     metadata_len: 0,
    ///     body_len: 0,
    ///     total_body_len: None,
    ///     is_last: true,
    /// });
    /// let parsed = ParsedFrameHeader::new(header, 0);
    /// assert!(matches!(parsed.into_header(), FrameHeader::First(_)));
    /// ```
    #[must_use]
    pub fn into_header(self) -> FrameHeader { self.header }
}
