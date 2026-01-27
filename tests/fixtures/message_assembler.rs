//! `MessageAssemblerWorld` fixture for rstest-bdd tests.
//!
//! Provides header parsing helpers for message assembler scenarios.

use std::{fmt, io};

use bytes::{BufMut, BytesMut};
use rstest::fixture;
use wireframe::{
    message_assembler::{FrameHeader, MessageAssembler, ParsedFrameHeader},
    test_helpers::TestAssembler,
};

use crate::TestApp;
// Re-export TestResult from common for use in steps
pub use crate::common::TestResult;

/// Specification for first-frame header encoding used in tests.
#[derive(Debug, Clone, Copy)]
pub struct FirstHeaderSpec {
    /// Message key to encode into the header.
    pub key: u64,
    /// Metadata length in bytes.
    pub metadata_len: usize,
    /// Body length in bytes for this frame.
    pub body_len: usize,
    /// Optional total body length across all frames.
    pub total_len: Option<usize>,
    /// Whether the frame is the final one in the series.
    pub is_last: bool,
}

impl FirstHeaderSpec {
    /// Create a first header spec with default metadata and flags.
    pub fn new(key: u64, body_len: usize) -> Self {
        Self {
            key,
            metadata_len: 0,
            body_len,
            total_len: None,
            is_last: false,
        }
    }

    /// Set the metadata length to encode into the header.
    pub fn with_metadata_len(mut self, metadata_len: usize) -> Self {
        self.metadata_len = metadata_len;
        self
    }

    /// Set the total message length to encode into the header.
    pub fn with_total_len(mut self, total_len: usize) -> Self {
        self.total_len = Some(total_len);
        self
    }

    /// Set whether the header should be marked as the final frame.
    pub fn with_last_flag(mut self, is_last: bool) -> Self {
        self.is_last = is_last;
        self
    }
}

/// Specification for continuation-frame header encoding used in tests.
#[derive(Debug, Clone, Copy)]
pub struct ContinuationHeaderSpec {
    /// Message key to encode into the header.
    pub key: u64,
    /// Body length in bytes for this frame.
    pub body_len: usize,
    /// Optional sequence number.
    pub sequence: Option<u32>,
    /// Whether the frame is the final one in the series.
    pub is_last: bool,
}

impl ContinuationHeaderSpec {
    /// Create a continuation header spec with default sequence and flags.
    pub fn new(key: u64, body_len: usize) -> Self {
        Self {
            key,
            body_len,
            sequence: None,
            is_last: false,
        }
    }

    /// Set the continuation sequence to encode into the header.
    pub fn with_sequence(mut self, sequence: u32) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Set whether the header should be marked as the final frame.
    pub fn with_last_flag(mut self, is_last: bool) -> Self {
        self.is_last = is_last;
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct HeaderEnvelope {
    kind: u8,
    flags: u8,
    key: u64,
}

/// Test world for message assembler header parsing.
#[derive(Default)]
pub struct MessageAssemblerWorld {
    payload: Option<Vec<u8>>,
    parsed: Option<ParsedFrameHeader>,
    error: Option<io::Error>,
    app: Option<TestApp>,
}

impl fmt::Debug for MessageAssemblerWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageAssemblerWorld")
            .field("payload", &self.payload)
            .field("parsed", &self.parsed)
            .field("error", &self.error)
            .field(
                "app",
                &self.app.as_ref().map(|_| "wireframe::app::WireframeApp"),
            )
            .finish()
    }
}

// rustfmt collapses simple fixtures into one line, which triggers unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn message_assembler_world() -> MessageAssemblerWorld {
    MessageAssemblerWorld::default()
}

impl MessageAssemblerWorld {
    /// Generic assertion helper for any header field.
    ///
    /// The extractor returns `Result<T, String>` to allow for both type-checking
    /// and field extraction. For fields present in both header types, the extractor
    /// should always succeed. For type-specific fields, the extractor can return an
    /// error if the header type is incorrect.
    fn assert_field<T, F>(&self, field_name: &str, expected: &T, extractor: F) -> TestResult
    where
        T: PartialEq + fmt::Display + Copy,
        F: FnOnce(&FrameHeader) -> Result<T, String>,
    {
        let parsed = self.parsed.as_ref().ok_or("no parsed header")?;
        let actual = extractor(parsed.header())?;
        if actual != *expected {
            return Err(format!("expected {field_name} {expected}, got {actual}").into());
        }
        Ok(())
    }

    /// Assert a field specific to First headers.
    fn assert_first_field<T, F>(&self, field_name: &str, expected: &T, extractor: F) -> TestResult
    where
        T: PartialEq + fmt::Display + Copy,
        F: FnOnce(&wireframe::message_assembler::FirstFrameHeader) -> T,
    {
        self.assert_field(field_name, expected, |header| {
            if let FrameHeader::First(header) = header {
                Ok(extractor(header))
            } else {
                Err("expected first header".to_string())
            }
        })
    }

    /// Assert a field specific to Continuation headers.
    fn assert_continuation_field<T, F>(
        &self,
        field_name: &str,
        expected: &T,
        extractor: F,
    ) -> TestResult
    where
        T: PartialEq + fmt::Display + Copy,
        F: FnOnce(&wireframe::message_assembler::ContinuationFrameHeader) -> T,
    {
        self.assert_field(field_name, expected, |header| {
            if let FrameHeader::Continuation(header) = header {
                Ok(extractor(header))
            } else {
                Err("expected continuation header".to_string())
            }
        })
    }

    /// Store an encoded first-frame header in the world payload.
    ///
    /// # Errors
    ///
    /// Returns an error if any length field exceeds the header encoding limits.
    pub fn set_first_header(&mut self, spec: FirstHeaderSpec) -> TestResult {
        let mut flags = 0u8;
        if spec.is_last {
            flags |= 0b1;
        }
        if spec.total_len.is_some() {
            flags |= 0b10;
        }
        self.set_payload_with_header(
            HeaderEnvelope {
                kind: 0x01,
                flags,
                key: spec.key,
            },
            |bytes| {
                let metadata_len =
                    u16::try_from(spec.metadata_len).map_err(|_| "metadata length too large")?;
                bytes.put_u16(metadata_len);
                let body_len = u32::try_from(spec.body_len).map_err(|_| "body length too large")?;
                bytes.put_u32(body_len);
                if let Some(total) = spec.total_len {
                    let total = u32::try_from(total).map_err(|_| "total length too large")?;
                    bytes.put_u32(total);
                }
                Ok(())
            },
        )
    }

    /// Store an encoded continuation-frame header in the world payload.
    ///
    /// # Errors
    ///
    /// Returns an error if any length field exceeds the header encoding limits.
    pub fn set_continuation_header(&mut self, spec: ContinuationHeaderSpec) -> TestResult {
        let mut flags = 0u8;
        if spec.is_last {
            flags |= 0b1;
        }
        if spec.sequence.is_some() {
            flags |= 0b10;
        }
        self.set_payload_with_header(
            HeaderEnvelope {
                kind: 0x02,
                flags,
                key: spec.key,
            },
            |bytes| {
                let body_len = u32::try_from(spec.body_len).map_err(|_| "body length too large")?;
                bytes.put_u32(body_len);
                if let Some(seq) = spec.sequence {
                    bytes.put_u32(seq);
                }
                Ok(())
            },
        )
    }

    fn set_payload_with_header<F>(&mut self, envelope: HeaderEnvelope, encode: F) -> TestResult
    where
        F: FnOnce(&mut BytesMut) -> TestResult,
    {
        let mut bytes = BytesMut::new();
        bytes.put_u8(envelope.kind);
        bytes.put_u8(envelope.flags);
        bytes.put_u64(envelope.key);
        encode(&mut bytes)?;
        self.payload = Some(bytes.to_vec());
        Ok(())
    }

    /// Store a deliberately invalid header payload.
    pub fn set_invalid_payload(&mut self) { self.payload = Some(vec![0x01]); }

    /// Parse the stored payload with the test assembler.
    ///
    /// # Errors
    ///
    /// Returns an error if no payload has been configured.
    pub fn parse_header(&mut self) -> TestResult {
        let payload = self.payload.as_deref().ok_or("payload not set")?;
        let fallback = TestAssembler;
        let assembler: &dyn MessageAssembler = match self.app.as_ref() {
            Some(app) => app
                .message_assembler()
                .ok_or("message assembler not set")?
                .as_ref(),
            None => &fallback,
        };
        match assembler.parse_frame_header(payload) {
            Ok(parsed) => {
                self.parsed = Some(parsed);
                self.error = None;
            }
            Err(err) => {
                self.parsed = None;
                self.error = Some(err);
            }
        }
        Ok(())
    }

    /// Assert that the parsed header is of the expected kind.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the kind does not match.
    pub fn assert_header_kind(&self, expected: &str) -> TestResult {
        let parsed = self.parsed.as_ref().ok_or("no parsed header")?;
        let matches_kind = matches!(
            (expected, parsed.header()),
            ("first", FrameHeader::First(_)) | ("continuation", FrameHeader::Continuation(_))
        );
        if matches_kind {
            Ok(())
        } else {
            Err(format!("expected {expected} header").into())
        }
    }
}

mod message_assembler_asserts;
