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
#[path = "common/server_harness.rs"]
mod server_harness;
#[path = "common/tagging_codec.rs"]
mod tagging_codec;

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use futures::stream;
use protocol_request::request_frame;
use recording_protocol::{HookEvent, MARKER, RecordingProtocolFixture};
use rstest::rstest;
use server_harness::{EVENT_TIMEOUT, read_frame, spawn_server, stop};
use tagging_codec::{TaggingCodec, WRAP_TAG};
use tokio::{
    net::TcpStream,
    sync::mpsc,
    time::{Instant, sleep, timeout},
};
use wireframe::{
    app::{Envelope, Handler, ResponseHandler},
    push::PushHandle,
    response::{FrameStream, Response},
    serializer::BincodeSerializer,
    session::SessionRegistry,
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

/// Handler that returns a two-frame multi-packet response.
///
/// The prepared connection's actor drains the channel, so these frames reach
/// the transport sink through the same pipeline as streamed responses.
fn two_frame_multi_packet_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let (tx, rx) = mpsc::channel(2);
            for payload in [vec![1_u8], vec![2_u8]] {
                // The receiver is returned in this response, so the channel
                // cannot be closed while these sends run.
                let _sent = tx.send(Envelope::new(2, None, payload)).await;
            }
            drop(tx);
            Response::MultiPacket(rx)
        })
    })
}

/// Wait until the prepared connection has dispatched `on_connection_setup`.
async fn wait_for_setup(fixture: &RecordingProtocolFixture<Bytes>) -> TestResult<()> {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    while fixture.events().is_empty() {
        if Instant::now() >= deadline {
            return Err("the prepared connection did not complete setup in time".into());
        }
        sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

/// Return the push handle of the single live connection.
fn only_registered_handle(registry: &SessionRegistry<Bytes>) -> TestResult<PushHandle<Bytes>> {
    let ids = registry.active_ids();
    let [id] = ids.as_slice() else {
        return Err(format!("expected exactly one live connection, got {}", ids.len()).into());
    };
    registry
        .get(id)
        .ok_or_else(|| "the live connection should publish its push handle".into())
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

/// A prepared multi-packet response dispatches setup, then runs `before_send`
/// once per channel frame and once on the terminator, in emission order.
#[rstest]
#[tokio::test]
async fn prepared_multi_packet_runs_before_send_once_per_frame_and_terminator() -> TestResult<()> {
    const TERMINATOR_BYTES: &[u8] = b"END";
    let fixture =
        RecordingProtocolFixture::<Bytes>::new(Some(Bytes::from_static(TERMINATOR_BYTES)));
    let app = ResponseApp::new()?
        .route_response(3, two_frame_multi_packet_handler())?
        .with_protocol(fixture.protocol());

    let bytes = prepare_and_drive_with_frames(app, vec![request_frame(3)?]).await?;

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
            "expected setup, two hooked channel frames, a hooked terminator, then completion, got \
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
        "each multi-packet frame and the terminator are hooked exactly once"
    );
    Ok(())
}

/// Pushes queued through a prepared connection's published handle are written
/// by the same pipeline as handler responses, so `before_send` runs once per
/// frame, in write order, and the peer decodes one marker per frame.
///
/// Each frame is read before the next push is queued, so the expected order
/// does not depend on scheduler timing. A push is not a handler response, so
/// the sequence carries no completion hook.
#[rstest]
#[tokio::test]
async fn prepared_pushes_run_before_send_once_in_write_order() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let registry = Arc::new(SessionRegistry::<Bytes>::new());
    let app = ResponseApp::new()?
        .with_protocol(fixture.protocol())
        .with_session_registry(Arc::clone(&registry));
    let (addr, shutdown) = spawn_server(app).await?;
    let mut client = TcpStream::connect(addr).await?;

    wait_for_setup(&fixture).await?;
    let handle = only_registered_handle(&registry)?;

    handle
        .push_high_priority(Bytes::from_static(b"high"))
        .await
        .map_err(|error| format!("high-priority push should be accepted: {error:?}"))?;
    let high = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;
    handle
        .push_low_priority(Bytes::from_static(b"low"))
        .await
        .map_err(|error| format!("low-priority push should be accepted: {error:?}"))?;
    let low = timeout(EVENT_TIMEOUT, read_frame(&mut client)).await??;

    let marked = |payload: &[u8]| {
        let mut frame = payload.to_vec();
        frame.push(MARKER);
        frame
    };
    assert_eq!(
        high,
        marked(b"high"),
        "the high-priority frame carries exactly one hook marker"
    );
    assert_eq!(
        low,
        marked(b"low"),
        "the low-priority frame carries exactly one hook marker"
    );
    assert_eq!(
        fixture.events(),
        vec![
            HookEvent::Setup,
            HookEvent::BeforeSend(Bytes::from_static(b"high")),
            HookEvent::BeforeSend(Bytes::from_static(b"low")),
        ],
        "the prepared push path dispatches setup, then one before_send per frame in write order"
    );

    drop(client);
    stop(&shutdown).await
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
