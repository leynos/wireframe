//! Test world for codec error taxonomy scenarios.
//!
//! Verifies that codec errors are correctly classified and that recovery
//! policies are applied as documented.
#![cfg(not(loom))]

use cucumber::World;
use wireframe::codec::{CodecError, EofError, FramingError, ProtocolError, RecoveryPolicy};

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

#[derive(Debug, Default, World)]
/// Test world for codec error taxonomy scenarios.
pub struct CodecErrorWorld {
    error_type: ErrorType,
    framing_variant: FramingVariant,
    eof_variant: EofVariant,
    current_error: Option<CodecError>,
    detected_eof: Option<EofError>,
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

    /// Record that a clean EOF was detected.
    pub fn record_clean_eof(&mut self) { self.detected_eof = Some(EofError::CleanClose); }

    /// Record that a mid-frame EOF was detected.
    pub fn record_mid_frame_eof(&mut self, bytes: usize) {
        self.detected_eof = Some(EofError::MidFrame {
            bytes_received: bytes,
            expected: 0,
        });
    }

    /// Verify that a clean EOF was detected.
    ///
    /// # Errors
    ///
    /// Returns an error if no EOF was detected or if a non-clean EOF was detected.
    pub fn verify_clean_eof(&self) -> TestResult {
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
            Some(EofError::MidFrame { .. }) => Ok(()),
            Some(other) => Err(format!("expected mid-frame EOF, got {other:?}").into()),
            None => Err("no EOF was detected".into()),
        }
    }
}
