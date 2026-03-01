//! Test world for codec fixture behavioural scenarios.
//!
//! Exercises the `wireframe_testing` codec fixture functions with
//! `HotlineFrameCodec` to verify that each fixture category produces
//! wire bytes with the expected decoding behaviour.

use std::io;

use rstest::fixture;
use wireframe::codec::{
    FrameCodec,
    examples::{HotlineFrame, HotlineFrameCodec},
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

/// BDD world holding the codec, decoded frames, and any decode error.
#[derive(Debug, Default)]
pub struct CodecFixturesWorld {
    codec: Option<HotlineFrameCodec>,
    decoded_frames: Vec<HotlineFrame>,
    decode_error: Option<io::Error>,
}

/// Fixture for codec fixture scenarios used by rstest-bdd steps.
///
/// Note: `rustfmt` collapses simple fixtures into one line, which triggers
/// `unused_braces`, so keep `rustfmt::skip`.
#[rustfmt::skip]
#[fixture]
pub fn codec_fixtures_world() -> CodecFixturesWorld {
    CodecFixturesWorld::default()
}

impl CodecFixturesWorld {
    /// Configure the Hotline codec with the given maximum frame length.
    ///
    /// # Errors
    /// Returns an error if the codec is already configured.
    pub fn configure_codec(&mut self, max_frame_length: usize) -> TestResult {
        if self.codec.is_some() {
            return Err("codec already configured".into());
        }
        self.codec = Some(HotlineFrameCodec::new(max_frame_length));
        Ok(())
    }

    /// Decode a valid fixture frame and store the results.
    ///
    /// # Errors
    /// Returns an error if the codec is not configured or decoding fails
    /// unexpectedly.
    pub fn decode_valid_fixture(&mut self) -> TestResult {
        let wire = wireframe_testing::valid_hotline_wire(b"fixture-payload", 7);
        self.decode_fixture(wire)
    }

    /// Decode an oversized fixture frame and store the error.
    ///
    /// # Errors
    /// Returns an error if the codec is not configured.
    pub fn decode_oversized_fixture(&mut self) -> TestResult {
        let codec = self.codec.as_ref().ok_or("codec not configured")?;
        let wire = wireframe_testing::oversized_hotline_wire(codec.max_frame_length());
        self.decode_fixture(wire)
    }

    /// Decode a truncated fixture frame and store the error.
    ///
    /// # Errors
    /// Returns an error if the codec is not configured.
    pub fn decode_truncated_fixture(&mut self) -> TestResult {
        let wire = wireframe_testing::truncated_hotline_header();
        self.decode_fixture(wire)
    }

    /// Decode correlated fixture frames and store the results.
    ///
    /// # Errors
    /// Returns an error if the codec is not configured or decoding fails
    /// unexpectedly.
    pub fn decode_correlated_fixture(&mut self) -> TestResult {
        let wire = wireframe_testing::correlated_hotline_wire(42, &[b"a", b"b", b"c"]);
        self.decode_fixture(wire)
    }

    /// Decode `wire` with the configured codec, storing frames or error.
    fn decode_fixture(&mut self, wire: Vec<u8>) -> TestResult {
        let codec = self.codec.as_ref().ok_or("codec not configured")?;
        match wireframe_testing::decode_frames_with_codec(codec, wire) {
            Ok(frames) => self.decoded_frames = frames,
            Err(e) => self.decode_error = Some(e),
        }
        Ok(())
    }

    /// Verify the decoded payload matches the expected fixture input.
    ///
    /// # Errors
    /// Returns an error if the assertion fails.
    pub fn verify_payload_matches(&self) -> TestResult {
        if self.decoded_frames.len() != 1 {
            return Err(format!("expected 1 frame, got {}", self.decoded_frames.len()).into());
        }
        let frame = self
            .decoded_frames
            .first()
            .ok_or("expected at least one frame")?;
        if frame.payload.as_ref() != b"fixture-payload" {
            return Err(format!(
                "payload mismatch: expected b\"fixture-payload\", got {:?}",
                frame.payload.as_ref()
            )
            .into());
        }
        Ok(())
    }

    /// Verify the decoder produced an `InvalidData` error for oversized
    /// frames.
    ///
    /// # Errors
    /// Returns an error if no decode error was captured or it does not
    /// contain the expected message.
    pub fn verify_invalid_data_error(&self) -> TestResult {
        self.verify_error_message_contains("payload too large")
    }

    /// Verify the decoder produced a "bytes remaining" error for truncated
    /// frames.
    ///
    /// # Errors
    /// Returns an error if no decode error was captured or it does not
    /// contain the expected message.
    pub fn verify_bytes_remaining_error(&self) -> TestResult {
        self.verify_error_message_contains("bytes remaining")
    }

    /// Verify the captured decode error contains `expected_substring`.
    fn verify_error_message_contains(&self, expected_substring: &str) -> TestResult {
        let err = self
            .decode_error
            .as_ref()
            .ok_or("expected a decode error but decoding succeeded")?;
        if !err.to_string().contains(expected_substring) {
            return Err(format!("expected '{expected_substring}' error, got: {err}").into());
        }
        Ok(())
    }

    /// Verify all decoded frames share the expected transaction identifier.
    ///
    /// # Errors
    /// Returns an error if no frames were decoded or any frame has a
    /// different transaction ID.
    pub fn verify_correlated_transaction_ids(&self) -> TestResult {
        if self.decoded_frames.len() != 3 {
            return Err(format!("expected 3 frames, got {}", self.decoded_frames.len()).into());
        }
        for (i, frame) in self.decoded_frames.iter().enumerate() {
            if frame.transaction_id != 42 {
                return Err(format!(
                    "frame {i}: expected transaction_id 42, got {}",
                    frame.transaction_id
                )
                .into());
            }
        }
        Ok(())
    }
}
