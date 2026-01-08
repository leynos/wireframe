//! Test world for codec error taxonomy scenarios.
//!
//! Verifies that codec errors are correctly classified and that recovery
//! policies are applied as documented. Uses real decoder operations for
//! end-to-end validation.
#![cfg(not(loom))]

use bytes::BytesMut;
use cucumber::World;
use tokio_util::codec::Decoder;
use wireframe::{
    FrameCodec,
    codec::{
        CodecError,
        EofError,
        FramingError,
        LengthDelimitedFrameCodec,
        ProtocolError,
        RecoveryPolicy,
    },
};

use super::TestResult;

/// Codec error type for test scenarios.
#[derive(Clone, Copy, Debug, Default)]
pub enum ErrorType {
    #[default]
    Framing,
    Protocol,
    Io,
    Eof,
}

/// Specific error variant for framing errors.
#[derive(Clone, Copy, Debug, Default)]
pub enum FramingVariant {
    #[default]
    Oversized,
    InvalidEncoding,
    IncompleteHeader,
    ChecksumMismatch,
    Empty,
}

/// Specific error variant for EOF errors.
#[derive(Clone, Copy, Debug, Default)]
pub enum EofVariant {
    #[default]
    CleanClose,
    MidFrame,
    MidHeader,
}

/// Test world for codec error taxonomy scenarios.
#[derive(Debug, Default, World)]
pub struct CodecErrorWorld {
    /// Current error type category being tested.
    error_type: ErrorType,
    /// Specific framing error variant when `error_type` is `Framing`.
    framing_variant: FramingVariant,
    /// Specific EOF error variant when `error_type` is `Eof`.
    eof_variant: EofVariant,
    /// Constructed error based on `error_type` and variant settings.
    current_error: Option<CodecError>,
    /// EOF error detected during decoder operations.
    detected_eof: Option<EofError>,
    /// Maximum frame length for the codec under test.
    max_frame_length: usize,
    /// Buffer simulating received data from a client.
    buffer: BytesMut,
    /// Decoder error captured during test.
    decoder_error: Option<std::io::Error>,
    /// Whether `decode_eof` returned `Ok(None)` for clean close.
    clean_close_detected: bool,
}

impl CodecErrorWorld {
    /// Set the current error type being tested.
    ///
    /// # Errors
    ///
    /// Returns an error if `error_type` is not one of: `framing`, `protocol`,
    /// `io`, or `eof`.
    pub fn set_error_type(&mut self, error_type: &str) -> TestResult {
        self.error_type = match error_type {
            "framing" => ErrorType::Framing,
            "protocol" => ErrorType::Protocol,
            "io" => ErrorType::Io,
            "eof" => ErrorType::Eof,
            _ => return Err(format!("unknown error type: {error_type}").into()),
        };
        self.build_error();
        Ok(())
    }

    /// Set the framing error variant.
    ///
    /// # Errors
    ///
    /// Returns an error if `variant` is not a recognized framing variant.
    pub fn set_framing_variant(&mut self, variant: &str) -> TestResult {
        self.framing_variant = match variant {
            "oversized" => FramingVariant::Oversized,
            "invalid_encoding" => FramingVariant::InvalidEncoding,
            "incomplete_header" => FramingVariant::IncompleteHeader,
            "checksum_mismatch" => FramingVariant::ChecksumMismatch,
            "empty" => FramingVariant::Empty,
            _ => return Err(format!("unknown framing variant: {variant}").into()),
        };
        self.build_error();
        Ok(())
    }

    /// Set the EOF error variant.
    ///
    /// # Errors
    ///
    /// Returns an error if `variant` is not a recognized EOF variant.
    pub fn set_eof_variant(&mut self, variant: &str) -> TestResult {
        self.eof_variant = match variant {
            "clean_close" => EofVariant::CleanClose,
            "mid_frame" => EofVariant::MidFrame,
            "mid_header" => EofVariant::MidHeader,
            _ => return Err(format!("unknown eof variant: {variant}").into()),
        };
        self.build_error();
        Ok(())
    }

    /// Build the current error based on type and variant settings.
    fn build_error(&mut self) {
        let error = match self.error_type {
            ErrorType::Framing => match self.framing_variant {
                FramingVariant::Oversized => CodecError::Framing(FramingError::OversizedFrame {
                    size: 2000,
                    max: 1024,
                }),
                FramingVariant::InvalidEncoding => {
                    CodecError::Framing(FramingError::InvalidLengthEncoding)
                }
                FramingVariant::IncompleteHeader => {
                    CodecError::Framing(FramingError::IncompleteHeader { have: 2, need: 4 })
                }
                FramingVariant::ChecksumMismatch => {
                    CodecError::Framing(FramingError::ChecksumMismatch {
                        expected: 0xdead,
                        actual: 0xbeef,
                    })
                }
                FramingVariant::Empty => CodecError::Framing(FramingError::EmptyFrame),
            },
            ErrorType::Protocol => {
                CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 99 })
            }
            ErrorType::Io => CodecError::Io(std::io::Error::other("test error")),
            ErrorType::Eof => match self.eof_variant {
                EofVariant::CleanClose => CodecError::Eof(EofError::CleanClose),
                EofVariant::MidFrame => CodecError::Eof(EofError::MidFrame {
                    bytes_received: 100,
                    expected: 200,
                }),
                EofVariant::MidHeader => CodecError::Eof(EofError::MidHeader {
                    bytes_received: 2,
                    header_size: 4,
                }),
            },
        };
        self.current_error = Some(error);
    }

    /// Verify the recovery policy for the current error.
    ///
    /// # Errors
    ///
    /// Returns an error if `expected` is not a recognized policy or if the
    /// actual policy does not match the expected policy.
    pub fn verify_recovery_policy(&self, expected: &str) -> TestResult {
        let expected_policy = match expected {
            "drop" => RecoveryPolicy::Drop,
            "quarantine" => RecoveryPolicy::Quarantine,
            "disconnect" => RecoveryPolicy::Disconnect,
            _ => return Err(format!("unknown recovery policy: {expected}").into()),
        };

        let error = self.current_error.as_ref().ok_or("no error has been set")?;
        let actual_policy = error.default_recovery_policy();

        if actual_policy != expected_policy {
            return Err(
                format!("expected policy {expected_policy:?}, got {actual_policy:?}").into(),
            );
        }

        Ok(())
    }

    // =========================================================================
    // Real E2E decoder operations
    // =========================================================================

    /// Reset codec state to prepare for a new test operation.
    fn reset_codec_state(&mut self) {
        self.buffer = BytesMut::new();
        self.decoder_error = None;
        self.clean_close_detected = false;
    }

    /// Configure the codec with default settings.
    pub fn setup_default_codec(&mut self) {
        self.max_frame_length = 1024;
        self.reset_codec_state();
    }

    /// Configure the codec with a specific max frame length.
    pub fn setup_codec_with_max_length(&mut self, max_len: usize) {
        self.max_frame_length = max_len;
        self.reset_codec_state();
    }

    /// Simulate a client sending a complete frame by encoding data into the buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if encoding fails.
    pub fn send_complete_frame(&mut self, payload: &[u8]) -> TestResult {
        use tokio_util::codec::Encoder;

        let codec = LengthDelimitedFrameCodec::new(self.max_frame_length);
        let mut encoder = codec.encoder();
        encoder.encode(bytes::Bytes::copy_from_slice(payload), &mut self.buffer)?;
        Ok(())
    }

    /// Simulate a client sending partial frame data (header only, no payload).
    pub fn send_partial_frame_header_only(&mut self) {
        // Write a length prefix indicating 100 bytes, but don't write any payload
        // 4-byte big-endian length prefix
        self.buffer.extend_from_slice(&[0x00, 0x00, 0x00, 0x64]); // 100 bytes expected
    }

    /// Call `decode_eof` to simulate a clean close at frame boundary.
    ///
    /// Returns `true` if `Ok(None)` was returned, indicating clean close.
    ///
    /// # Errors
    ///
    /// Returns an error if clean close was not detected.
    pub fn decode_eof_clean_close(&mut self) -> TestResult {
        let codec = LengthDelimitedFrameCodec::new(self.max_frame_length);
        let mut decoder = codec.decoder();

        // First decode any complete frames
        while let Some(_frame) = decoder.decode(&mut self.buffer)? {
            // Consume complete frames
        }

        // Now call decode_eof to handle EOF
        match decoder.decode_eof(&mut self.buffer) {
            Ok(None) => {
                self.clean_close_detected = true;
                self.detected_eof = Some(EofError::CleanClose);
                Ok(())
            }
            Ok(Some(_)) => Err("unexpected frame after EOF".into()),
            Err(e) => {
                self.decoder_error = Some(e);
                Err("expected clean close, got error".into())
            }
        }
    }

    /// Length prefix header size (4 bytes for big-endian u32).
    const LENGTH_HEADER_SIZE: usize = 4;

    /// Extract the expected payload length from the buffer's length header.
    ///
    /// Returns 0 if the buffer doesn't contain a complete 4-byte header.
    #[expect(
        clippy::big_endian_bytes,
        reason = "Wire protocol uses big-endian length prefix; this matches the codec."
    )]
    fn extract_expected_length(&self) -> usize {
        self.buffer
            .get(..Self::LENGTH_HEADER_SIZE)
            .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
            .map_or(0, |bytes| u32::from_be_bytes(bytes) as usize)
    }

    /// Classify the EOF error type from the error message.
    ///
    /// # Implementation Note
    ///
    /// This method infers EOF type by checking if the error message contains
    /// "header". This is fragile: if the upstream error message format changes,
    /// this classification will silently produce incorrect results.
    ///
    /// When the buffer contains at least 4 bytes (a complete length header),
    /// we extract the expected payload length from the big-endian u32 prefix.
    ///
    /// This approach is acceptable for test code where we control the error
    /// messages, but would need a more robust solution (e.g., downcasting to
    /// `CodecError`) if the underlying error type becomes available.
    fn classify_eof_error(&mut self, e: &std::io::Error) {
        if e.kind() != std::io::ErrorKind::UnexpectedEof {
            return;
        }
        let msg = e.to_string();
        self.detected_eof = Some(if msg.contains("header") {
            EofError::MidHeader {
                bytes_received: self.buffer.len(),
                header_size: Self::LENGTH_HEADER_SIZE,
            }
        } else {
            EofError::MidFrame {
                bytes_received: self.buffer.len().saturating_sub(Self::LENGTH_HEADER_SIZE),
                expected: self.extract_expected_length(),
            }
        });
    }

    /// Call `decode_eof` when buffer has incomplete data.
    ///
    /// # Errors
    ///
    /// Returns an error if no EOF error was produced.
    pub fn decode_eof_with_partial_data(&mut self) -> TestResult {
        let codec = LengthDelimitedFrameCodec::new(self.max_frame_length);
        let mut decoder = codec.decoder();

        match decoder.decode_eof(&mut self.buffer) {
            Ok(None) => Err("expected EOF error, got Ok(None)".into()),
            Ok(Some(_)) => Err("expected EOF error, got frame".into()),
            Err(e) => {
                self.classify_eof_error(&e);
                self.decoder_error = Some(e);
                Ok(())
            }
        }
    }

    /// Attempt to encode an oversized frame.
    ///
    /// # Errors
    ///
    /// Returns an error if no oversized error was produced.
    pub fn encode_oversized_frame(&mut self, size: usize) -> TestResult {
        use tokio_util::codec::Encoder;

        let codec = LengthDelimitedFrameCodec::new(self.max_frame_length);
        let mut encoder = codec.encoder();
        let payload = bytes::Bytes::from(vec![0_u8; size]);

        match encoder.encode(payload, &mut self.buffer) {
            Ok(()) => Err("expected oversized error, got Ok".into()),
            Err(e) => {
                self.decoder_error = Some(e);
                Ok(())
            }
        }
    }

    /// Verify that a clean EOF was detected.
    ///
    /// # Errors
    ///
    /// Returns an error if no EOF was detected or if a non-clean EOF was detected.
    pub fn verify_clean_eof(&self) -> TestResult {
        if self.clean_close_detected {
            return Ok(());
        }
        match &self.detected_eof {
            Some(EofError::CleanClose) => Ok(()),
            Some(other) => Err(format!("expected clean close, got {other:?}").into()),
            None => Err("no EOF was detected".into()),
        }
    }

    /// Verify that a mid-frame EOF was detected.
    ///
    /// # Errors
    ///
    /// Returns an error if no EOF was detected or if it was not a mid-frame EOF.
    pub fn verify_mid_frame_eof(&self) -> TestResult {
        match &self.detected_eof {
            Some(EofError::MidFrame { .. } | EofError::MidHeader { .. }) => Ok(()),
            Some(other) => Err(format!("expected mid-frame EOF, got {other:?}").into()),
            None => Err("no EOF was detected".into()),
        }
    }

    /// Verify that an oversized frame error was detected.
    ///
    /// # Errors
    ///
    /// Returns an error if no error was captured or if it wasn't an oversized error.
    pub fn verify_oversized_error(&self) -> TestResult {
        let err = self
            .decoder_error
            .as_ref()
            .ok_or("no decoder error captured")?;
        if err.kind() == std::io::ErrorKind::InvalidData {
            // OversizedFrame is converted to InvalidData
            Ok(())
        } else {
            Err(format!("expected InvalidData error, got {:?}", err.kind()).into())
        }
    }
}
