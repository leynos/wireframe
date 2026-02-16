//! End-to-end decoder operations for codec error taxonomy tests.
//!
//! Provides real decoder operations to validate EOF error handling and frame
//! encoding/decoding behaviour in realistic scenarios.

use bytes::BytesMut;
use tokio_util::codec::Decoder;
use wireframe::{
    FrameCodec,
    byte_order::{read_network_u32, write_network_u32},
    codec::{EofError, LENGTH_HEADER_SIZE, LengthDelimitedFrameCodec},
};

use super::{CodecErrorWorld, TestResult};

impl CodecErrorWorld {
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
        self.buffer.extend_from_slice(&write_network_u32(100)); // 100 bytes expected
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

    /// Extract the expected payload length from the buffer's length header.
    ///
    /// Returns 0 if the buffer doesn't contain a complete length header.
    fn extract_expected_length(&self) -> usize {
        self.buffer
            .get(..LENGTH_HEADER_SIZE)
            .and_then(|slice| <[u8; LENGTH_HEADER_SIZE]>::try_from(slice).ok())
            .map_or(0, |bytes| read_network_u32(bytes) as usize)
    }

    /// Classify the EOF error type from the inner error.
    ///
    /// # Implementation Note
    ///
    /// The decoder wraps `EofError` inside `io::Error` for
    /// `UnexpectedEof`. We downcast to recover the precise variant. If the
    /// inner error is missing, fall back to inspecting the buffer length to
    /// infer whether we stopped mid-header or mid-frame.
    fn classify_eof_error(&mut self, e: &std::io::Error) {
        if e.kind() != std::io::ErrorKind::UnexpectedEof {
            return;
        }
        let detected = e
            .get_ref()
            .and_then(Self::find_eof_error)
            .unwrap_or_else(|| self.infer_eof_from_buffer());
        self.detected_eof = Some(detected);
    }

    fn find_eof_error(error: &(dyn std::error::Error + Send + Sync + 'static)) -> Option<EofError> {
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
        while let Some(err) = current {
            if let Some(eof) = err.downcast_ref::<EofError>() {
                return Some(*eof);
            }
            current = err.source();
        }
        None
    }

    fn infer_eof_from_buffer(&self) -> EofError {
        if self.buffer.len() < LENGTH_HEADER_SIZE {
            EofError::MidHeader {
                bytes_received: self.buffer.len(),
                header_size: LENGTH_HEADER_SIZE,
            }
        } else {
            EofError::MidFrame {
                bytes_received: self.buffer.len().saturating_sub(LENGTH_HEADER_SIZE),
                expected: self.extract_expected_length(),
            }
        }
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

    /// Verify that an incomplete EOF was detected (either mid-frame or mid-header).
    ///
    /// # Errors
    ///
    /// Returns an error if no EOF was detected or if it was a clean close.
    pub fn verify_incomplete_eof(&self) -> TestResult {
        match &self.detected_eof {
            Some(EofError::MidFrame { .. } | EofError::MidHeader { .. }) => Ok(()),
            Some(other) => Err(format!("expected incomplete EOF, got {other:?}").into()),
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

#[cfg(test)]
mod tests {
    //! Tests for decoder ops EOF classification and buffer inference behaviour.

    use bytes::BufMut;
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    fn codec_error_world() -> CodecErrorWorld {
        let mut world = CodecErrorWorld::default();
        world.reset_codec_state();
        world
    }

    #[rstest]
    #[case::clean_close(EofError::CleanClose)]
    #[case::mid_header(EofError::MidHeader {
        bytes_received: 1,
        header_size: LENGTH_HEADER_SIZE,
    })]
    #[case::mid_frame(EofError::MidFrame {
        bytes_received: 2,
        expected: 3,
    })]
    fn classify_eof_error_uses_inner_eof_error_variant(
        #[case] variant: EofError,
        mut codec_error_world: CodecErrorWorld,
    ) {
        let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, variant);

        codec_error_world.classify_eof_error(&io_err);

        assert_eq!(codec_error_world.detected_eof, Some(variant));
    }

    #[rstest]
    fn classify_eof_error_falls_back_to_buffer_classification(
        mut codec_error_world: CodecErrorWorld,
    ) {
        let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "not an eof error");
        codec_error_world.buffer.extend_from_slice(&[0x01, 0x02]);
        let expected = codec_error_world.infer_eof_from_buffer();

        codec_error_world.classify_eof_error(&io_err);

        assert_eq!(codec_error_world.detected_eof, Some(expected));
    }

    #[rstest]
    fn classify_eof_error_ignores_non_unexpected_eof(mut codec_error_world: CodecErrorWorld) {
        let io_err = std::io::Error::other("other error");
        codec_error_world.detected_eof = Some(EofError::CleanClose);

        codec_error_world.classify_eof_error(&io_err);

        assert_eq!(codec_error_world.detected_eof, Some(EofError::CleanClose));
    }

    #[rstest]
    fn infer_eof_from_buffer_reports_mid_header(mut codec_error_world: CodecErrorWorld) {
        codec_error_world.buffer.extend_from_slice(&[0x01, 0x02]);

        match codec_error_world.infer_eof_from_buffer() {
            EofError::MidHeader {
                bytes_received,
                header_size,
            } => {
                assert_eq!(bytes_received, 2);
                assert_eq!(header_size, LENGTH_HEADER_SIZE);
            }
            other => panic!("expected MidHeader, got {other:?}"),
        }
    }

    #[rstest]
    fn infer_eof_from_buffer_reports_mid_frame(mut codec_error_world: CodecErrorWorld) {
        let expected_len: u32 = 42;
        let expected_usize = usize::try_from(expected_len).expect("expected length fits in usize");
        codec_error_world.buffer.put_u32(expected_len);
        codec_error_world.buffer.extend_from_slice(&[0x11, 0x22]);

        match codec_error_world.infer_eof_from_buffer() {
            EofError::MidFrame {
                bytes_received,
                expected,
            } => {
                assert_eq!(
                    bytes_received,
                    codec_error_world
                        .buffer
                        .len()
                        .saturating_sub(LENGTH_HEADER_SIZE)
                );
                assert_eq!(expected, expected_usize);
            }
            other => panic!("expected MidFrame, got {other:?}"),
        }
    }
}
