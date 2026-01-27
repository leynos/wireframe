//! `CodecErrorWorld` fixture for rstest-bdd tests.
//!
//! Verifies codec error taxonomy and recovery policy defaults.

mod decoder_ops;

use bytes::BytesMut;
use rstest::fixture;
use wireframe::codec::{CodecError, EofError, FramingError, ProtocolError, RecoveryPolicy};

/// `TestResult` for step definitions.
pub use crate::common::TestResult;

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
#[derive(Debug, Default)]
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
    pub(crate) detected_eof: Option<EofError>,
    /// Maximum frame length for the codec under test.
    pub(crate) max_frame_length: usize,
    /// Buffer simulating received data from a client.
    pub(crate) buffer: BytesMut,
    /// Decoder error captured during test.
    pub(crate) decoder_error: Option<std::io::Error>,
    /// Whether `decode_eof` returned `Ok(None)` for clean close.
    pub(crate) clean_close_detected: bool,
}

/// Fixture for `CodecErrorWorld`.
#[rustfmt::skip]
#[fixture]
pub fn codec_error_world() -> CodecErrorWorld {
    CodecErrorWorld::default()
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
    /// Returns an error if `variant` is not a recognised framing variant.
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
    /// Returns an error if `variant` is not a recognised EOF variant.
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
        self.current_error = Some(match self.error_type {
            ErrorType::Framing => CodecError::Framing(self.build_framing_error()),
            ErrorType::Protocol => {
                CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 99 })
            }
            ErrorType::Io => CodecError::Io(std::io::Error::other("test error")),
            ErrorType::Eof => CodecError::Eof(self.build_eof_error()),
        });
    }

    /// Build a framing error based on the current variant.
    fn build_framing_error(&self) -> FramingError {
        match self.framing_variant {
            FramingVariant::Oversized => FramingError::OversizedFrame {
                size: 2000,
                max: 1024,
            },
            FramingVariant::InvalidEncoding => FramingError::InvalidLengthEncoding,
            FramingVariant::IncompleteHeader => FramingError::IncompleteHeader { have: 2, need: 4 },
            FramingVariant::ChecksumMismatch => FramingError::ChecksumMismatch {
                expected: 0xdead,
                actual: 0xbeef,
            },
            FramingVariant::Empty => FramingError::EmptyFrame,
        }
    }

    /// Build an EOF error based on the current variant.
    fn build_eof_error(&self) -> EofError {
        match self.eof_variant {
            EofVariant::CleanClose => EofError::CleanClose,
            EofVariant::MidFrame => EofError::MidFrame {
                bytes_received: 100,
                expected: 200,
            },
            EofVariant::MidHeader => EofError::MidHeader {
                bytes_received: 2,
                header_size: 4,
            },
        }
    }

    /// Verify the recovery policy for the current error.
    ///
    /// # Errors
    ///
    /// Returns an error if `expected` is not a recognised policy or if the
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
}
