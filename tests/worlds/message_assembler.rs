//! Test world for message assembler header parsing.
#![cfg(not(loom))]

use std::{fmt, io};

use bytes::{Buf, BufMut, BytesMut};
use cucumber::World;
use wireframe::message_assembler::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameHeader,
    FrameSequence,
    MessageAssembler,
    MessageKey,
    ParsedFrameHeader,
};

use super::TestResult;

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

/// World used by Cucumber to test message assembler header parsing.
#[derive(Debug, Default, World)]
pub struct MessageAssemblerWorld {
    payload: Option<Vec<u8>>,
    parsed: Option<ParsedFrameHeader>,
    error: Option<io::Error>,
}

impl MessageAssemblerWorld {
    fn assert_common_field<T, F>(&self, field: &str, expected: T, extractor: F) -> TestResult
    where
        T: PartialEq + fmt::Display,
        F: FnOnce(&FrameHeader) -> T,
    {
        let parsed = self.parsed.as_ref().ok_or("no parsed header")?;
        let actual = extractor(parsed.header());
        if actual != expected {
            return Err(format!("expected {field} {expected}, got {actual}").into());
        }
        Ok(())
    }

    fn assert_first_field<T, F>(&self, field_name: &str, expected: T, extractor: F) -> TestResult
    where
        T: PartialEq + fmt::Display,
        F: FnOnce(&FirstFrameHeader) -> T,
    {
        let parsed = self.parsed.as_ref().ok_or("no parsed header")?;
        let FrameHeader::First(header) = parsed.header() else {
            return Err("expected first header".into());
        };
        let actual = extractor(header);
        if actual != expected {
            return Err(format!("expected {field_name} {expected}, got {actual}").into());
        }
        Ok(())
    }

    fn assert_continuation_field<T, F>(
        &self,
        field_name: &str,
        expected: T,
        extractor: F,
    ) -> TestResult
    where
        T: PartialEq + fmt::Display,
        F: FnOnce(&ContinuationFrameHeader) -> T,
    {
        let parsed = self.parsed.as_ref().ok_or("no parsed header")?;
        let FrameHeader::Continuation(header) = parsed.header() else {
            return Err("expected continuation header".into());
        };
        let actual = extractor(header);
        if actual != expected {
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
        let mut bytes = BytesMut::new();
        bytes.put_u8(0x01);
        let mut flags = 0u8;
        if spec.is_last {
            flags |= 0b1;
        }
        if spec.total_len.is_some() {
            flags |= 0b10;
        }
        bytes.put_u8(flags);
        bytes.put_u64(spec.key);
        let metadata_len =
            u16::try_from(spec.metadata_len).map_err(|_| "metadata length too large")?;
        bytes.put_u16(metadata_len);
        let body_len = u32::try_from(spec.body_len).map_err(|_| "body length too large")?;
        bytes.put_u32(body_len);
        if let Some(total) = spec.total_len {
            let total = u32::try_from(total).map_err(|_| "total length too large")?;
            bytes.put_u32(total);
        }
        self.payload = Some(bytes.to_vec());
        Ok(())
    }

    /// Store an encoded continuation-frame header in the world payload.
    ///
    /// # Errors
    ///
    /// Returns an error if any length field exceeds the header encoding limits.
    pub fn set_continuation_header(&mut self, spec: ContinuationHeaderSpec) -> TestResult {
        let mut bytes = BytesMut::new();
        bytes.put_u8(0x02);
        let mut flags = 0u8;
        if spec.is_last {
            flags |= 0b1;
        }
        if spec.sequence.is_some() {
            flags |= 0b10;
        }
        bytes.put_u8(flags);
        bytes.put_u64(spec.key);
        let body_len = u32::try_from(spec.body_len).map_err(|_| "body length too large")?;
        bytes.put_u32(body_len);
        if let Some(seq) = spec.sequence {
            bytes.put_u32(seq);
        }
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
        match TestAssembler.parse_frame_header(payload) {
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
        self.assert_common_field("key", expected, |header| match header {
            FrameHeader::First(header) => u64::from(header.message_key),
            FrameHeader::Continuation(header) => u64::from(header.message_key),
        })
    }

    /// Assert that the parsed header contains the expected metadata length.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the metadata length differs.
    pub fn assert_metadata_len(&self, expected: usize) -> TestResult {
        self.assert_first_field("metadata length", expected, |header| header.metadata_len)
    }

    /// Assert that the parsed header contains the expected body length.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the body length differs.
    pub fn assert_body_len(&self, expected: usize) -> TestResult {
        self.assert_common_field("body length", expected, |header| match header {
            FrameHeader::First(header) => header.body_len,
            FrameHeader::Continuation(header) => header.body_len,
        })
    }

    /// Assert that the parsed header contains the expected total body length.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the total length differs.
    pub fn assert_total_len(&self, expected: Option<usize>) -> TestResult {
        self.assert_first_field(
            "total length",
            DebugDisplay(expected),
            |header| DebugDisplay(header.total_body_len),
        )
    }

    /// Assert that the parsed header contains the expected sequence.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the sequence differs.
    pub fn assert_sequence(&self, expected: Option<u32>) -> TestResult {
        let expected = expected.map(FrameSequence::from);
        self.assert_continuation_field(
            "sequence",
            DebugDisplay(expected),
            |header| DebugDisplay(header.sequence),
        )
    }

    /// Assert that the parsed header matches the expected `is_last` flag.
    ///
    /// # Errors
    ///
    /// Returns an error if no header was parsed or the flag differs.
    pub fn assert_is_last(&self, expected: bool) -> TestResult {
        self.assert_common_field("is_last", expected, |header| match header {
            FrameHeader::First(header) => header.is_last,
            FrameHeader::Continuation(header) => header.is_last,
        })
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DebugDisplay<T>(T);

impl<T: fmt::Debug> fmt::Display for DebugDisplay<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{:?}", self.0) }
}

struct TestAssembler;

impl MessageAssembler for TestAssembler {
    fn parse_frame_header(&self, payload: &[u8]) -> Result<ParsedFrameHeader, io::Error> {
        let mut buf = payload;
        let initial = buf.remaining();
        let kind = take_u8(&mut buf)?;
        let flags = take_u8(&mut buf)?;
        let message_key = MessageKey::from(take_u64(&mut buf)?);

        let header = match kind {
            0x01 => {
                let metadata_len = usize::from(take_u16(&mut buf)?);
                let body_len = usize::try_from(take_u32(&mut buf)?)
                    .map_err(|_| invalid_data("body length too large"))?;
                let total_body_len = if flags & 0b10 == 0b10 {
                    Some(
                        usize::try_from(take_u32(&mut buf)?)
                            .map_err(|_| invalid_data("total length too large"))?,
                    )
                } else {
                    None
                };
                FrameHeader::First(FirstFrameHeader {
                    message_key,
                    metadata_len,
                    body_len,
                    total_body_len,
                    is_last: flags & 0b1 == 0b1,
                })
            }
            0x02 => {
                let body_len = usize::try_from(take_u32(&mut buf)?)
                    .map_err(|_| invalid_data("body length too large"))?;
                let sequence = if flags & 0b10 == 0b10 {
                    Some(FrameSequence::from(take_u32(&mut buf)?))
                } else {
                    None
                };
                FrameHeader::Continuation(ContinuationFrameHeader {
                    message_key,
                    sequence,
                    body_len,
                    is_last: flags & 0b1 == 0b1,
                })
            }
            _ => return Err(invalid_data("unknown header kind")),
        };

        let header_len = initial - buf.remaining();
        Ok(ParsedFrameHeader::new(header, header_len))
    }
}

fn take_u8(buf: &mut &[u8]) -> Result<u8, io::Error> {
    ensure_remaining(buf, 1)?;
    Ok(buf.get_u8())
}

fn take_u16(buf: &mut &[u8]) -> Result<u16, io::Error> {
    ensure_remaining(buf, 2)?;
    Ok(buf.get_u16())
}

fn take_u32(buf: &mut &[u8]) -> Result<u32, io::Error> {
    ensure_remaining(buf, 4)?;
    Ok(buf.get_u32())
}

fn take_u64(buf: &mut &[u8]) -> Result<u64, io::Error> {
    ensure_remaining(buf, 8)?;
    Ok(buf.get_u64())
}

fn ensure_remaining(buf: &mut &[u8], needed: usize) -> Result<(), io::Error> {
    if buf.remaining() < needed {
        return Err(invalid_data("header too short"));
    }
    Ok(())
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
