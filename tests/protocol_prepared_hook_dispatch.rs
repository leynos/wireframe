//! Hook-dispatch boundary tests for the prepared-application paths.
//!
//! A prepared connection serializes the `Envelope`, wraps it with the codec,
//! and only then runs `before_send`, immediately before the transport write.
//! These tests pin that boundary and the prepared path's lifecycle contract by
//! driving real prepared connections, not by invoking hooks directly. The
//! directly driven actor paths live in `protocol_hook_dispatch`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/protocol_request.rs"]
mod protocol_request;
#[path = "common/recording_protocol.rs"]
mod recording_protocol;
#[path = "common/tagging_codec.rs"]
mod tagging_codec;

use std::sync::Arc;

use bytes::Bytes;
use futures::stream;
use protocol_request::request_frame;
use recording_protocol::{HookEvent, MARKER, RecordingProtocolFixture};
use rstest::rstest;
use tagging_codec::{TaggingCodec, WRAP_TAG};
use wireframe::{
    app::{Envelope, Handler, ResponseHandler},
    response::{FrameStream, Response},
    serializer::BincodeSerializer,
};
use wireframe_testing::{TestResult, decode_frames, prepare_and_drive_with_frames};

/// Application type using the default length-delimited codec (`Bytes` frames).
type ResponseApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

/// Handler that accepts an envelope without changing it.
fn echo_handler() -> Handler<Envelope> { Arc::new(|_env: &Envelope| Box::pin(async {})) }

/// Handler that streams two frames before completing.
fn two_frame_stream_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let frames: FrameStream<Envelope, ()> = Box::pin(stream::iter(vec![
                Ok(Envelope::new(2, None, vec![1_u8])),
                Ok(Envelope::new(2, None, vec![2_u8])),
            ]));
            Response::Stream(frames)
        })
    })
}

/// The hook observes the codec-wrapped frame, after the serializer and the
/// codec have run and immediately before the transport write.
///
/// That is the boundary ADR 012's 2026-11-18 addendum records: the hook takes
/// `&mut F::Frame`, so it cannot see the `Envelope` before serialization, and
/// moving it there would reopen ADR 010. Setup is dispatched once before the
/// response, so it leads the observed order.
#[rstest]
#[tokio::test]
async fn prepared_response_runs_before_send_once_after_serialization() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let app = ResponseApp::new()?
        .route(1, echo_handler())?
        .with_codec(TaggingCodec::new(1024))
        .with_protocol(fixture.protocol());

    let bytes = prepare_and_drive_with_frames(app, vec![request_frame(1)?]).await?;

    let frames = decode_frames(&bytes)?;
    let [decoded] = frames.as_slice() else {
        return Err(format!("expected exactly one response frame, got {}", frames.len()).into());
    };
    let events = fixture.events();
    let [
        HookEvent::Setup,
        HookEvent::BeforeSend(observed),
        HookEvent::CommandEnd,
    ] = events.as_slice()
    else {
        return Err(format!(
            "expected setup, before_send, then command end on the prepared path, got {events:?}"
        )
        .into());
    };

    assert_eq!(
        observed.last(),
        Some(&WRAP_TAG),
        "the hook must run after the codec wrapped the payload: {observed:?}"
    );
    let mut expected = observed.to_vec();
    expected.push(MARKER);
    assert_eq!(
        decoded, &expected,
        "the hook must observe the codec-wrapped frame that the peer decodes"
    );
    Ok(())
}

/// A prepared stream dispatches setup, then runs `before_send` once per frame
/// and once on the terminator, in emission order.
#[rstest]
#[tokio::test]
async fn prepared_stream_runs_before_send_once_per_frame_and_terminator() -> TestResult<()> {
    const TERMINATOR_BYTES: &[u8] = b"END";
    let fixture =
        RecordingProtocolFixture::<Bytes>::new(Some(Bytes::from_static(TERMINATOR_BYTES)));
    let app = ResponseApp::new()?
        .route_response(2, two_frame_stream_handler())?
        .with_protocol(fixture.protocol());

    let bytes = prepare_and_drive_with_frames(app, vec![request_frame(2)?]).await?;

    let events = fixture.events();
    let [
        HookEvent::Setup,
        HookEvent::BeforeSend(first),
        HookEvent::BeforeSend(second),
        HookEvent::StreamEnd,
        HookEvent::BeforeSend(terminator),
        HookEvent::CommandEnd,
    ] = events.as_slice()
    else {
        return Err(format!(
            "expected setup, two hooked frames, a hooked terminator, then completion, got \
             {events:?}"
        )
        .into());
    };
    assert_eq!(
        &terminator[..],
        TERMINATOR_BYTES,
        "the terminator reaches the hook as the protocol supplied it"
    );
    let expected: Vec<Vec<u8>> = [first, second, terminator]
        .into_iter()
        .map(|frame| {
            let mut marked = frame.to_vec();
            marked.push(MARKER);
            marked
        })
        .collect();
    assert_eq!(
        decode_frames(&bytes)?,
        expected,
        "each streamed frame and the terminator are hooked exactly once"
    );
    Ok(())
}

/// A prepared connection dispatches `on_connection_setup` exactly once and
/// never `on_eof`, including when the peer closes mid-frame.
///
/// Setup runs at the connection boundary, because that is where the push
/// handle and the registry entry come into existence; the reference used for
/// that call is not retained, so the connection still holds exactly one strong
/// protocol reference. The end-of-file callback is outside the prepared
/// contract: a mid-frame close is reported to the caller as an I/O error.
/// ADR 012's 2026-11-20 addendum records that split.
#[rstest]
#[tokio::test]
async fn prepared_connection_dispatches_setup_once_and_no_eof_hook() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let app = ResponseApp::new()?.with_protocol(fixture.protocol());

    // One byte is a truncated length header, so the peer's close is a mid-frame
    // end-of-file while the protocol is installed. The runtime reports it to the
    // caller as an I/O error; the protocol must not be told about it.
    let outcome = prepare_and_drive_with_frames(app, vec![vec![0x00]]).await;
    assert!(
        outcome.is_err(),
        "a truncated length header must reach the caller: {outcome:?}"
    );

    let events = fixture.events();
    assert_eq!(
        events,
        vec![HookEvent::Setup],
        "a prepared connection dispatches setup once and never EOF: {events:?}"
    );
    Ok(())
}
