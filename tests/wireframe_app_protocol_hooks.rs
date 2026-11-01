//! Integration tests for protocol hooks on application handler responses.
//!
//! These tests prove that a [`WireframeProtocol`] installed with
//! `with_protocol` observes, and can mutate, every response frame produced by
//! `WireframeApp` handlers on both the prepared and unprepared connection
//! paths.
#![cfg(not(loom))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;
use rstest::rstest;
use tokio_util::codec::LengthDelimitedCodec;
use wireframe::{
    app::{Envelope, Handler},
    hooks::{ConnectionContext, WireframeProtocol},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    TestResult,
    decode_frames,
    drive_with_frames,
    encode_frame,
    prepare_and_drive_with_frames,
};

/// Application type using the default length-delimited codec (`Bytes` frames).
type ResponseApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

/// Marker appended to application responses by the test protocol.
const APP_MARKER: u8 = 0xb7;

/// Protocol whose `before_send` marks every outbound `Bytes` frame.
struct ResponseMarkerProtocol {
    /// Number of frames the hook has observed.
    calls: Arc<AtomicUsize>,
}

impl WireframeProtocol for ResponseMarkerProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn before_send(&self, frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {
        let mut bytes = frame.to_vec();
        bytes.push(APP_MARKER);
        *frame = Bytes::from(bytes);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

/// Handler that accepts an envelope and returns an empty response payload.
fn echo_handler() -> Handler<Envelope> { Arc::new(|_env: &Envelope| Box::pin(async {})) }

/// Encode one request envelope into a length-delimited frame.
fn request_frame(id: u32) -> TestResult<Vec<u8>> {
    let payload = BincodeSerializer.serialize(&Envelope::new(id, Some(7), Vec::new()))?;
    let mut codec = LengthDelimitedCodec::builder()
        .max_frame_length(1024)
        .new_codec();
    Ok(encode_frame(&mut codec, payload)?)
}

/// Decode response frames, stripping the protocol marker from each payload.
fn decode_marked_payloads(bytes: &[u8]) -> TestResult<Vec<Vec<u8>>> {
    decode_frames(bytes)?
        .into_iter()
        .map(|frame| {
            let Some((&marker, payload)) = frame.split_last() else {
                return Err("response frame was empty".into());
            };
            if marker != APP_MARKER {
                return Err(
                    format!("response frame missing protocol marker: got {marker:#04x}").into(),
                );
            }
            let (envelope, consumed) = BincodeSerializer.deserialize::<Envelope>(payload)?;
            if consumed != payload.len() {
                return Err("trailing bytes after response envelope".into());
            }
            Ok(envelope.payload_bytes().to_vec())
        })
        .collect()
}

#[rstest]
#[tokio::test]
async fn prepared_app_runs_before_send_for_handler_responses() -> TestResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = ResponseApp::new()?
        .route(1, echo_handler())?
        .with_protocol(ResponseMarkerProtocol {
            calls: Arc::clone(&calls),
        });

    let bytes = prepare_and_drive_with_frames(app, vec![request_frame(1)?]).await?;

    assert_eq!(
        decode_marked_payloads(&bytes)?,
        vec![Vec::<u8>::new()],
        "the response should carry the protocol marker"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "before_send should run for the handler response"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn unprepared_app_runs_before_send_for_handler_responses() -> TestResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = ResponseApp::new()?
        .route(1, echo_handler())?
        .with_protocol(ResponseMarkerProtocol {
            calls: Arc::clone(&calls),
        });

    let bytes = drive_with_frames(app, vec![request_frame(1)?]).await?;

    assert_eq!(
        decode_marked_payloads(&bytes)?,
        vec![Vec::<u8>::new()],
        "the compatibility path should also apply before_send"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "before_send should run on the unprepared path"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn before_send_runs_for_every_response_on_one_connection() -> TestResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = ResponseApp::new()?
        .route(1, echo_handler())?
        .with_protocol(ResponseMarkerProtocol {
            calls: Arc::clone(&calls),
        });

    let bytes =
        prepare_and_drive_with_frames(app, vec![request_frame(1)?, request_frame(1)?]).await?;

    assert_eq!(
        decode_marked_payloads(&bytes)?.len(),
        2,
        "both responses should be marked on the same connection"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "before_send should run once per response"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn responses_without_a_protocol_are_not_marked() -> TestResult<()> {
    let app = ResponseApp::new()?.route(1, echo_handler())?;

    let bytes = prepare_and_drive_with_frames(app, vec![request_frame(1)?]).await?;

    let frames = decode_frames(&bytes)?;
    let [frame] = frames.as_slice() else {
        return Err(format!("expected one response frame, got {}", frames.len()).into());
    };
    let (envelope, consumed) = BincodeSerializer.deserialize::<Envelope>(frame)?;
    assert_eq!(
        consumed,
        frame.len(),
        "no protocol marker should be appended without a protocol"
    );
    assert_eq!(envelope.payload_bytes(), []);
    Ok(())
}
