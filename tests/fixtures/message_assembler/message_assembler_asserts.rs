//! Assertion helpers for `MessageAssemblerWorld`.
//!
//! This module keeps the fixture file under the 400 line guideline while
//! preserving a cohesive set of assertion helpers.

use std::{fmt, io};

use wireframe::{
    message_assembler::{FrameHeader, FrameSequence},
    test_helpers::TestAssembler,
};

use super::{MessageAssemblerWorld, TestApp, TestResult};

#[derive(Clone, Copy, Debug, PartialEq)]
struct DebugDisplay<T>(T);

impl<T: fmt::Debug> fmt::Display for DebugDisplay<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{:?}", self.0) }
}

impl MessageAssemblerWorld {
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
        self.assert_first_field("metadata length", &expected, |header| header.metadata_len)
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
        self.assert_first_field("total length", &expected, |header| {
            DebugDisplay(header.total_body_len)
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
        self.assert_continuation_field("sequence", &expected, |header| {
            DebugDisplay(header.sequence)
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
