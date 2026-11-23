//! Structured end-of-file conditions for the bundled example codecs.
//!
//! The Hotline and `MySQL` adapters are the worked examples of custom framing,
//! so they must opt in to the same `on_eof` contract as the bundled
//! length-delimited decoder. A peer that closes mid-header or mid-payload has
//! to produce `EofError::MidHeader` or `EofError::MidFrame` from `decode_eof`,
//! leaving the partial bytes in the buffer for the connection to report.
//!
//! A malformed header is a framing error, not a truncation, and a close at a
//! frame boundary is a clean close. Both must stay outside the EOF taxonomy,
//! or a protocol would be told that data was lost when none was.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::io;

use bytes::{BufMut, Bytes, BytesMut};
use rstest::rstest;
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{
    CodecError,
    EofError,
    FrameCodec,
    examples::{HotlineFrame, HotlineFrameCodec, MysqlFrame, MysqlFrameCodec},
};

/// Maximum frame length the success cases allow.
const MAX_FRAME_LENGTH: usize = 64;

/// Which bundled example codec a case exercises.
#[derive(Clone, Copy, Debug)]
enum BundledCodec {
    Hotline,
    Mysql,
}

impl BundledCodec {
    /// Decoder presenting each frame's payload in place of the frame.
    fn decoder(self, max_frame_length: usize) -> Box<dyn Decoder<Item = Bytes, Error = io::Error>> {
        match self {
            Self::Hotline => Box::new(PayloadDecoder(
                HotlineFrameCodec::new(max_frame_length).decoder(),
            )),
            Self::Mysql => Box::new(PayloadDecoder(
                MysqlFrameCodec::new(max_frame_length).decoder(),
            )),
        }
    }

    /// Number of bytes in this codec's frame header.
    const fn header_len(self) -> usize {
        match self {
            Self::Hotline => 20,
            Self::Mysql => 4,
        }
    }

    /// Encode one frame whose payload is `payload_len` bytes of `0x7`.
    fn wire(self, payload_len: usize) -> BytesMut {
        let payload = Bytes::from(vec![7_u8; payload_len]);
        match self {
            Self::Hotline => encode(
                HotlineFrameCodec::new(MAX_FRAME_LENGTH).encoder(),
                HotlineFrame {
                    transaction_id: 1,
                    payload,
                },
            ),
            Self::Mysql => encode(
                MysqlFrameCodec::new(MAX_FRAME_LENGTH).encoder(),
                MysqlFrame {
                    sequence_id: 1,
                    payload,
                },
            ),
        }
    }
}

/// A decoded example frame whose payload a case can inspect.
trait Payload {
    /// Borrow the decoded payload.
    fn payload(&self) -> &Bytes;
}

impl Payload for HotlineFrame {
    fn payload(&self) -> &Bytes { &self.payload }
}

impl Payload for MysqlFrame {
    fn payload(&self) -> &Bytes { &self.payload }
}

/// Adapts an example decoder to yield only each frame's payload.
struct PayloadDecoder<D>(D);

impl<D, F> Decoder for PayloadDecoder<D>
where
    D: Decoder<Item = F, Error = io::Error>,
    F: Payload,
{
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(self.0.decode(src)?.map(|frame| frame.payload().clone()))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(self.0.decode_eof(src)?.map(|frame| frame.payload().clone()))
    }
}

/// Extract the structured end-of-file condition a decode error carries.
fn eof_condition(error: &io::Error) -> Option<EofError> {
    error
        .get_ref()?
        .downcast_ref::<CodecError>()
        .and_then(|codec_error| match codec_error {
            CodecError::Eof(eof) => Some(*eof),
            _ => None,
        })
}

/// Encode one frame and return its wire bytes.
fn encode<F>(mut encoder: impl Encoder<F, Error = io::Error>, frame: F) -> BytesMut {
    let mut buffer = BytesMut::new();
    encoder
        .encode(frame, &mut buffer)
        .expect("encode a complete frame");
    buffer
}

/// A Hotline header whose declared frame length contradicts its payload length.
/// A Hotline header whose declared frame length contradicts its payload length.
fn hotline_bad_header() -> BytesMut {
    let mut buffer = BytesMut::new();
    buffer.put_u32(1);
    buffer.put_u32(1);
    buffer.put_u32(7);
    buffer.extend_from_slice(&[0_u8; 8]);
    buffer
}

/// Run an incomplete decode and assert that `decode_eof` reports `expected`.
fn assert_truncation(
    decoder: &mut dyn Decoder<Item = Bytes, Error = io::Error>,
    buffer: &mut BytesMut,
    expected: EofError,
) {
    assert!(
        decoder
            .decode(buffer)
            .expect("an incomplete frame is not an error")
            .is_none()
    );
    let error = decoder
        .decode_eof(buffer)
        .expect_err("a truncated frame must be an error");
    assert_eq!(eof_condition(&error), Some(expected));
}

/// How a case leaves a frame incomplete.
#[derive(Clone, Copy, Debug)]
enum Incomplete {
    /// Only part of the header arrived.
    Header { received: usize },
    /// The whole header arrived but part of the payload did not.
    Payload { received: usize },
}

#[rstest]
#[case::hotline_header(
    BundledCodec::Hotline,
    Incomplete::Header { received: 7 },
    EofError::MidHeader {
        bytes_received: 7,
        header_size: 20,
    }
)]
#[case::mysql_header(
    BundledCodec::Mysql,
    Incomplete::Header { received: 3 },
    EofError::MidHeader {
        bytes_received: 3,
        header_size: 4,
    }
)]
#[case::hotline_payload(
    BundledCodec::Hotline,
    Incomplete::Payload { received: 3 },
    EofError::MidFrame {
        bytes_received: 3,
        expected: 8,
    }
)]
#[case::mysql_payload(
    BundledCodec::Mysql,
    Incomplete::Payload { received: 5 },
    EofError::MidFrame {
        bytes_received: 5,
        expected: 8,
    }
)]
fn decode_eof_reports_a_partial_frame(
    #[case] codec: BundledCodec,
    #[case] incomplete: Incomplete,
    #[case] expected: EofError,
) {
    let mut decoder = codec.decoder(MAX_FRAME_LENGTH);
    let mut buffer = match incomplete {
        Incomplete::Header { received } => BytesMut::from(vec![0_u8; received].as_slice()),
        Incomplete::Payload { received } => {
            let mut wire = codec.wire(8);
            wire.truncate(codec.header_len() + received);
            wire
        }
    };
    let partial_len = buffer.len();

    assert_truncation(decoder.as_mut(), &mut buffer, expected);
    assert_eq!(
        buffer.len(),
        partial_len,
        "the partial frame must not be consumed"
    );
}

#[rstest]
#[case::hotline(BundledCodec::Hotline)]
#[case::mysql(BundledCodec::Mysql)]
fn decode_eof_is_clean_at_a_frame_boundary(#[case] codec: BundledCodec) {
    let mut decoder = codec.decoder(MAX_FRAME_LENGTH);
    let mut buffer = BytesMut::new();

    assert!(
        decoder
            .decode_eof(&mut buffer)
            .expect("a clean close is not an error")
            .is_none()
    );
}

#[rstest]
#[case::hotline(BundledCodec::Hotline)]
#[case::mysql(BundledCodec::Mysql)]
fn decode_eof_decodes_a_complete_frame(#[case] codec: BundledCodec) {
    let mut decoder = codec.decoder(MAX_FRAME_LENGTH);
    let mut buffer = codec.wire(5);

    let payload = decoder
        .decode_eof(&mut buffer)
        .expect("a complete frame decodes")
        .expect("the frame is present");
    assert_eq!(payload, Bytes::from(vec![7_u8; 5]));
    assert!(buffer.is_empty(), "a complete frame is consumed");
}

#[rstest]
#[case::hotline(BundledCodec::Hotline, MAX_FRAME_LENGTH, hotline_bad_header())]
#[case::mysql(BundledCodec::Mysql, 3, BytesMut::from(&[4_u8, 0, 0, 1][..]))]
fn decode_eof_keeps_framing_errors(
    #[case] codec: BundledCodec,
    #[case] max_frame_length: usize,
    #[case] mut buffer: BytesMut,
) {
    let mut decoder = codec.decoder(max_frame_length);

    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("a malformed header must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        eof_condition(&error),
        None,
        "a framing error must not be reported as a truncation"
    );
}
