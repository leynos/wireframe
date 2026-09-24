//! Tests for Wireframe inbound connection handling.

use bytes::{Bytes, BytesMut};
use tokio::io::AsyncWriteExt;
use tokio_util::codec::{Decoder, Encoder};
use wireframe_testing::logger;

use super::*;
use crate::{app::frame_handling, serializer::BincodeSerializer};

#[derive(Clone, Debug)]
struct BadFrame {
    correlation_id: u64,
    payload: Vec<u8>,
}

#[derive(Clone, Debug)]
struct BadCodec;

#[derive(Clone, Debug)]
struct BadDecoder;

#[derive(Clone, Debug)]
struct BadEncoder;

impl Decoder for BadDecoder {
    type Item = BadFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            Ok(None)
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "bad test frame"))
        }
    }
}

impl Encoder<BadFrame> for BadEncoder {
    type Error = io::Error;

    fn encode(&mut self, _item: BadFrame, _dst: &mut BytesMut) -> Result<(), Self::Error> { Ok(()) }
}

impl FrameCodec for BadCodec {
    type Frame = BadFrame;
    type Decoder = BadDecoder;
    type Encoder = BadEncoder;

    fn decoder(&self) -> Self::Decoder { BadDecoder }

    fn encoder(&self) -> Self::Encoder { BadEncoder }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        BadFrame {
            correlation_id: 0,
            payload: payload.to_vec(),
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(frame.correlation_id) }

    fn max_frame_length(&self) -> usize { 64 }
}

#[test]
fn decode_envelope_tracks_failures_and_logs_correlation_id() {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build app")
        .with_codec(BadCodec);
    let frame = BadFrame {
        correlation_id: 42,
        payload: vec![0xff],
    };

    let mut log = logger();
    log.clear();
    let mut deser_failures = 0_u32;

    for _ in 1..MAX_DESER_FAILURES {
        let mut failure_tracker =
            frame_handling::DeserFailureTracker::new(&mut deser_failures, MAX_DESER_FAILURES);
        let result = frame_handling::decode_envelope::<BadCodec>(
            core::parse_envelope(&app.serializer, BadCodec::frame_payload(&frame)),
            &frame,
            &mut failure_tracker,
        );
        let decoded = result.expect("expected recoverable decode failure");
        assert!(
            decoded.is_none(),
            "recoverable decode should not yield a message"
        );
    }

    let mut failure_tracker =
        frame_handling::DeserFailureTracker::new(&mut deser_failures, MAX_DESER_FAILURES);
    let err = frame_handling::decode_envelope::<BadCodec>(
        core::parse_envelope(&app.serializer, BadCodec::frame_payload(&frame)),
        &frame,
        &mut failure_tracker,
    )
    .expect_err("expected decode failure to close connection");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);

    let mut found = false;
    while let Some(record) = log.pop() {
        let message = record.args().to_string();
        if message.contains("failed to decode message")
            && message.contains("correlation_id=Some(42)")
        {
            found = true;
            break;
        }
    }
    assert!(found, "expected correlation_id in decode failure log");
}

#[tokio::test]
async fn unprepared_connection_preserves_stream_errors_and_diagnostics() {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build app")
        .with_codec(BadCodec);
    let (mut client, server) = tokio::io::duplex(64);
    client.write_all(&[1]).await.expect("write invalid frame");
    drop(client);

    let mut log = logger();
    log.clear();
    let error = app
        .handle_unprepared_connection_result(server)
        .await
        .expect_err("invalid frame must fail the connection");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    let mut found = false;
    while let Some(record) = log.pop() {
        let message = record.args();
        let has_error_context = [
            "connection terminated with error",
            "correlation_id=None",
            "bad test frame",
        ]
        .iter()
        .all(|&fragment| message.contains(fragment));
        if record.level() == log::Level::Warn && has_error_context {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "connection error must retain its warning and context"
    );
}
