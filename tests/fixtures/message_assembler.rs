//! `MessageAssemblerWorld` fixture for rstest-bdd tests.
//!
//! Provides header parsing helpers for message assembler scenarios.

#![expect(unused_braces, reason = "rustfmt forces single-line fixture functions")]

use std::{fmt, io};

use bytes::{BufMut, BytesMut};
use rstest::fixture;
use wireframe::{
    message_assembler::{FrameHeader, FrameSequence, MessageAssembler, ParsedFrameHeader},
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

#[fixture]
pub fn message_assembler_world() -> MessageAssemblerWorld { MessageAssemblerWorld::default() }

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

    /// Assert that the parsed header contains the expected message key.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the key does not match.
    pub fn assert_message_key(&self, expected: u64) -> TestResult {
        self.assert_field("key", &expected, |header| {
            Ok(match header {
                FrameHeader::First(header) => u64::from(header.message_key),
                FrameHeader::Continuation(header) => u64::from(header.message_key),
            })
        })
    }

    /// Assert that the parsed header contains the expected metadata length.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the metadata length differs.
    pub fn assert_metadata_len(&self, expected: usize) -> TestResult {
        self.assert_field("metadata length", &expected, |header| {
            if let FrameHeader::First(header) = header {
                Ok(header.metadata_len)
            } else {
                Err("expected first header".to_string())
            }
        })
    }

    /// Assert that the parsed header contains the expected body length.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the body length differs.
    pub fn assert_body_len(&self, expected: usize) -> TestResult {
        self.assert_field("body length", &expected, |header| {
            Ok(match header {
                FrameHeader::First(header) => header.body_len,
                FrameHeader::Continuation(header) => header.body_len,
            })
        })
    }

    /// Assert that the parsed header contains the expected total body length.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the total length differs.
    pub fn assert_total_len(&self, expected: Option<usize>) -> TestResult {
        let expected = DebugDisplay(expected);
        self.assert_field("total length", &expected, |header| {
            if let FrameHeader::First(header) = header {
                Ok(DebugDisplay(header.total_body_len))
            } else {
                Err("expected first header".to_string())
            }
        })
    }

    /// Assert that the parsed header contains the expected sequence.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the sequence differs.
    pub fn assert_sequence(&self, expected: Option<u32>) -> TestResult {
        let expected = expected.map(FrameSequence::from);
        let expected = DebugDisplay(expected);
        self.assert_field("sequence", &expected, |header| {
            if let FrameHeader::Continuation(header) = header {
                Ok(DebugDisplay(header.sequence))
            } else {
                Err("expected continuation header".to_string())
            }
        })
    }

    /// Assert that the parsed header matches the expected `is_last` flag.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the flag differs.
    pub fn assert_is_last(&self, expected: bool) -> TestResult {
        self.assert_field("is_last", &expected, |header| {
            Ok(match header {
                FrameHeader::First(header) => header.is_last,
                FrameHeader::Continuation(header) => header.is_last,
            })
        })
    }

    /// Assert that the parsed header length matches the expected value.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the length differs.
    pub fn assert_header_len(&self, expected: usize) -> TestResult {
        let parsed = self.parsed.as_ref().ok_or("no parsed header")?;
        let actual = parsed.header_len();
        if actual != expected {
            return Err(format!("expected header length {expected}, got {actual}").into());
        }
        Ok(())
    }

    /// Assert that the parse failed with `InvalidData`.
    ///
    /// # Errors
    ///
    /// Returns an error if no parse error was captured or the kind differs.
    pub fn assert_invalid_data_error(&self) -> TestResult {
        let err = self.error.as_ref().ok_or("expected error")?;
        if err.kind() != io::ErrorKind::InvalidData {
            return Err(format!("expected InvalidData error, got {:?}", err.kind()).into());
        }
        Ok(())
    }

    /// Store a wireframe app configured with a test message assembler.
    ///
    /// # Errors
    ///
    /// Returns an error if the app builder fails.
    pub fn set_app_with_message_assembler(&mut self) -> TestResult {
        let app = TestApp::new()
            .map_err(|err| format!("failed to build app: {err}"))?
            .with_message_assembler(TestAssembler);
        self.app = Some(app);
        Ok(())
    }

    /// Assert that the app exposes a message assembler.
    ///
    /// # Errors
    ///
    /// Returns an error if the app or assembler is missing.
    pub fn assert_message_assembler_configured(&self) -> TestResult {
        let app = self.app.as_ref().ok_or("app not set")?;
        if app.message_assembler().is_some() {
            Ok(())
        } else {
            Err("expected message assembler".into())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DebugDisplay<T>(T);

impl<T: fmt::Debug> fmt::Display for DebugDisplay<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{:?}", self.0) }
}
