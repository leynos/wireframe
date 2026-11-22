//! A truncation is reported once, whichever wait observed the read error.
//!
//! The connection loop used to report a truncation only for an error from its
//! own read wait, so a read error observed while a handler's response or a
//! push write was pending propagated straight out of `handle_step` and the
//! protocol `on_eof` hook never ran. Both paths now end through the loop's
//! single error boundary, and these scenarios pin each of them:
//!
//! - [`truncation_during_a_pending_handler_is_reported`] has the peer vanish while the response is
//!   still pending, so the error comes from the peer watcher.
//! - [`truncation_during_a_blocked_push_write_is_reported`] has the peer vanish while a push frame
//!   cannot be written, so the error comes from the push write's own read arm.

use std::{sync::Arc, time::Duration};

use bytes::{Bytes, BytesMut};
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{Notify, oneshot},
    time::timeout,
};
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{EofError, LENGTH_HEADER_SIZE, LengthDelimitedFrameCodec},
    push::PushHandle,
    response::Response,
    serializer::BincodeSerializer,
};
use wireframe_testing::{TestError, TestResult};

use super::harness::{
    ROUTE,
    STEP_TIMEOUT,
    await_connection_end,
    build_app,
    build_request,
    ended_notifier,
    ensure,
    single_handler,
    start_server,
};
use crate::hook_recorder::{Event, HookLog, RecordingProtocol};

/// Bytes a peer sends before it disconnects, part-way through a header.
const PARTIAL_HEADER: [u8; 2] = [0x00, 0x10];
/// Frame length the stalled-write codec accepts.
const STALL_CODEC_MAX: usize = 8 * 1024 * 1024;
/// Payload of the push whose write must stay pending.
const STALLED_PUSH_LEN: usize = STALL_CODEC_MAX - LENGTH_HEADER_SIZE;
/// Leading bytes of the stalled push payload.
const STALLED_PUSH_PREFIX: &[u8] = b"STALLED-PUSH";
/// How often a bounded wait rechecks the setup handle.
const HANDLE_POLL: Duration = Duration::from_millis(10);
/// Window a completing push write is given to surface in the hook log.
const PENDING_WRITE_WINDOW: Duration = Duration::from_millis(100);

/// A truncation read while a handler is pending reaches `on_eof` once.
///
/// The handler never answers, so the only thing that can end the connection is
/// the read error the peer watcher observes alongside the pending response.
#[tokio::test]
async fn truncation_during_a_pending_handler_is_reported() -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = oneshot::channel();
    let started = Arc::new(Notify::new());
    let app = build_app(&log, None, ended_tx, pending_response(Arc::clone(&started)))?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    client.write_all(&build_request(ROUTE)?).await?;
    timeout(STEP_TIMEOUT, started.notified())
        .await
        .map_err(|_| TestError::Msg("the handler never started".into()))?;
    client.write_all(&PARTIAL_HEADER).await?;
    client.shutdown().await?;
    await_connection_end(ended_rx).await?;

    let events = log.events();
    ensure(
        events == vec![Event::Setup, truncated_event()],
        format!("a truncation during a pending response must be reported once: {events:?}"),
    )?;

    drop(client);
    server.stop().await
}

/// A truncation read while a push write is blocked reaches `on_eof` once.
///
/// The peer never reads, so a push of [`STALLED_PUSH_LEN`] bytes cannot be
/// written: it exceeds the kernel's send buffer. A second push waits behind it
/// in the one-slot lane, and its acceptance proves the first frame was
/// dequeued and its write started. A bounded window then checks the hook log
/// still holds only the first frame, so the write is still pending when the
/// truncation arrives. The truncation therefore ends the connection from
/// inside that write, and the second frame is never written.
#[tokio::test]
async fn truncation_during_a_blocked_push_write_is_reported() -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = oneshot::channel();
    let app = stalling_app(&log, ended_tx)?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    let handle = await_push_handle(&log).await?;
    push(&handle, stalled_push_payload()).await?;
    // The lane holds one frame, so this push only completes once the
    // connection has taken the stalled frame and started writing it.
    push(&handle, Bytes::from_static(b"QUEUED-BEHIND")).await?;
    // A write that finished would surface as the second frame's `BeforeSend`
    // within this window, so this asserts the pending-write precondition
    // rather than assuming it.
    tokio::time::sleep(PENDING_WRITE_WINDOW).await;
    let pending = log.events();
    ensure(
        matches!(pending.as_slice(), [Event::Setup, Event::BeforeSend(_)]),
        format!(
            "the stalled push write must still be pending: {}",
            describe(&pending)
        ),
    )?;
    client.write_all(&PARTIAL_HEADER).await?;
    client.shutdown().await?;
    await_connection_end(ended_rx).await?;

    let events = log.events();
    match events.as_slice() {
        [
            Event::Setup,
            Event::BeforeSend(frame),
            Event::Eof { error, partial },
        ] => {
            ensure(
                frame.starts_with(STALLED_PUSH_PREFIX),
                format!("the stalled push must be the frame being written: {frame:?}"),
            )?;
            ensure(
                *error
                    == EofError::MidHeader {
                        bytes_received: PARTIAL_HEADER.len(),
                        header_size: LENGTH_HEADER_SIZE,
                    },
                format!("the truncation must be reported exactly: {error:?}"),
            )?;
            ensure(
                partial.as_slice() == PARTIAL_HEADER,
                format!("the partial frame must be reported exactly: {partial:?}"),
            )?;
        }
        other => {
            return Err(TestError::Msg(format!(
                "expected setup, the stalled push, and one truncation: {}",
                describe(other)
            )));
        }
    }

    drop(client);
    server.stop().await
}

/// The truncation event a two-byte header abandoned mid-frame produces.
fn truncated_event() -> Event {
    Event::Eof {
        error: EofError::MidHeader {
            bytes_received: PARTIAL_HEADER.len(),
            header_size: LENGTH_HEADER_SIZE,
        },
        partial: PARTIAL_HEADER.to_vec(),
    }
}

/// Describe events by kind, so a failure never dumps a large push payload.
fn describe(events: &[Event]) -> String {
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| match event {
            Event::Setup => "setup",
            Event::BeforeSend(_) => "before_send",
            Event::StreamEnd => "stream_end",
            Event::CommandEnd => "command_end",
            Event::ProtocolError => "protocol_error",
            Event::Eof { .. } => "eof",
        })
        .collect();
    format!("{kinds:?}")
}

/// Handler that announces its start and never produces a response.
///
/// Only a read error can end the connection, which is what keeps the peer
/// watcher, rather than a finished response, the observer of the truncation.
fn pending_response(started: Arc<Notify>) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let notifier = Arc::clone(&started);
        Box::pin(async move {
            notifier.notify_one();
            std::future::pending::<()>().await;
            Response::Single(Envelope::new(ROUTE, None, vec![9]))
        })
    })
}

/// Build the application whose codec accepts the stalled push frame.
fn stalling_app(
    log: &Arc<HookLog>,
    ended: oneshot::Sender<()>,
) -> TestResult<WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>> {
    Ok(WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route_response(ROUTE, single_handler())?
        .with_codec(LengthDelimitedFrameCodec::new(STALL_CODEC_MAX))
        .with_protocol(RecordingProtocol::new(Arc::clone(log), None))
        .on_connection_setup(|| async {})?
        .on_connection_teardown(ended_notifier(ended))?)
}

/// Wait until connection setup has published its push handle.
async fn await_push_handle(log: &HookLog) -> TestResult<PushHandle<Bytes>> {
    timeout(STEP_TIMEOUT, async {
        loop {
            if let Some(handle) = log.push_handle() {
                return handle;
            }
            tokio::time::sleep(HANDLE_POLL).await;
        }
    })
    .await
    .map_err(|_| TestError::Msg("the connection never ran its setup hook".into()))
}

/// Push one frame, reporting a rejected push as a test failure.
async fn push(handle: &PushHandle<Bytes>, frame: Bytes) -> TestResult {
    handle
        .push_high_priority(frame)
        .await
        .map_err(|error| TestError::Msg(format!("the connection rejected a push: {error}")))
}

/// A push payload larger than any transport buffer will absorb.
///
/// The prefix makes the frame identifiable in the hook log without dumping the
/// multi-megabyte payload; the rest is filler.
fn stalled_push_payload() -> Bytes {
    let mut payload = BytesMut::with_capacity(STALLED_PUSH_LEN);
    payload.extend_from_slice(STALLED_PUSH_PREFIX);
    payload.resize(STALLED_PUSH_LEN, b'A');
    payload.freeze()
}
