//! Truncated frames reach `on_eof` with the bytes that actually arrived.
//!
//! A peer can abandon a frame in three shapes: part of the length header, a
//! complete header with no payload, and a header with part of its payload. The
//! structured condition distinguishes them, and the partial data is exactly
//! what the framed reader still held, length prefix included, so a protocol can
//! account for the frame it lost.
//!
//! The hook reports only what the connection observes. A response that ends the
//! connection stops the read loop while the peer may still be mid-frame, and
//! that later disconnect is not a truncation the connection ever saw.

use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
};

use bytes::{Bytes, BytesMut};
use futures::stream;
use rstest::rstest;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{Notify, oneshot},
    time::timeout,
};
use tokio_util::codec::Decoder;
use wireframe::{
    WireframeError,
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{EofError, FrameCodec, LengthDelimitedDecoder, LengthDelimitedFrameCodec},
    response::{FrameStream, Response},
    serializer::BincodeSerializer,
};
use wireframe_testing::TestResult;

use super::harness::{
    ROUTE,
    STEP_TIMEOUT,
    await_connection_end,
    build_app,
    build_request,
    client_codec,
    ended_notifier,
    ensure,
    single_handler,
    start_server,
};
use crate::hook_recorder::{Event, HookLog, RecordingProtocol};

/// A disconnect inside a frame reports the truncation and its exact bytes.
#[rstest]
#[case::mid_header(&[0x00, 0x10], EofError::MidHeader { bytes_received: 2, header_size: 4 })]
#[case::header_only(&[0, 0, 0, 8], EofError::MidFrame { bytes_received: 0, expected: 8 })]
#[case::partial_payload(
    &[0, 0, 0, 8, 1, 2, 3],
    EofError::MidFrame { bytes_received: 3, expected: 8 }
)]
#[tokio::test]
async fn truncation_reports_the_partial_frame(
    #[case] wire: &[u8],
    #[case] expected: EofError,
) -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    let app = build_app(&log, None, ended_tx, single_handler())?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    client.write_all(wire).await?;
    client.shutdown().await?;
    await_connection_end(ended_rx).await?;

    let events = log.events();
    ensure(
        events
            == vec![
                Event::Setup,
                Event::Eof {
                    error: expected,
                    partial: wire.to_vec(),
                },
            ],
        format!("a truncated frame must be reported exactly: {events:?}"),
    )?;

    drop(client);
    server.stop().await
}

/// A response that ends the connection leaves the peer's tail unobserved.
///
/// The contract reports a truncation only when the connection reads it. Here
/// the first response fails, which stops dispatch while the read half has
/// already buffered a complete pipelined request and the opening bytes of the
/// next frame. The peer then half-closes, and the connection never reads again,
/// so `on_eof` must not run for bytes it never saw.
///
/// The evidence that the pipelined request was read while the response was
/// still pending comes from the codec, not a delay: a decoder that hands over a
/// complete frame signals the test, which then releases the failing response.
#[tokio::test]
async fn unread_truncation_is_not_reported() -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let frames = Arc::new(FrameWatch::default());
    let app = build_observing_app(
        &log,
        ended_tx,
        failing_after(Arc::clone(&started), Arc::clone(&release)),
        Arc::clone(&frames),
    )?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    client.write_all(&build_request(ROUTE)?).await?;
    // Dispatch is inside the handler, so the connection is already polling its
    // read half alongside the pending response.
    timeout(STEP_TIMEOUT, started.notified())
        .await
        .map_err(|_| "the handler never started")?;

    // The watch is armed before the write, so a decode that happens as soon as
    // the request lands cannot be missed. The partial header rides in the same
    // write, so the peer is genuinely mid-frame when it half-closes.
    let buffered = frames.watch();
    let mut pipelined = build_request(ROUTE)?;
    pipelined.extend_from_slice(&[0x00, 0x10]);
    client.write_all(&pipelined).await?;
    timeout(STEP_TIMEOUT, buffered)
        .await
        .map_err(|_| "the connection never read the pipelined request")?
        .map_err(|_| "the frame watch was dropped")?;
    client.shutdown().await?;

    release.notify_one();
    await_connection_end(ended_rx).await?;

    let events = log.events();
    ensure(
        events.contains(&Event::ProtocolError),
        format!("the failed response must reach handle_error: {events:?}"),
    )?;
    ensure(
        !events
            .iter()
            .any(|event| matches!(event, Event::Eof { .. })),
        format!("a truncation the connection never read must not be reported: {events:?}"),
    )?;

    drop(client);
    server.stop().await
}

/// One-shot signal that the connection decoded a frame after the test asked.
///
/// The connection stops polling its read half once one frame is buffered, so a
/// test cannot otherwise prove that a pipelined request reached the connection
/// while an earlier response was still pending.
#[derive(Default)]
struct FrameWatch {
    /// Sender armed by [`Self::watch`], taken by the first decoded frame.
    sender: Mutex<Option<oneshot::Sender<()>>>,
}

impl FrameWatch {
    /// Arm the watch and return the receiver for the next decoded frame.
    fn watch(&self) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        *self.sender.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
        receiver
    }

    /// Signal that one frame reached the connection's decoder.
    fn frame_decoded(&self) {
        let pending = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(sender) = pending {
            let _sent = sender.send(());
        }
    }
}

/// Length-delimited codec that signals when the connection decodes a frame.
///
/// Everything is forwarded to the bundled codec, so the wire format and the
/// truncation contract stay exactly those of [`LengthDelimitedFrameCodec`].
#[derive(Clone)]
struct ObservingCodec {
    /// Bundled codec that owns the framing rules.
    inner: LengthDelimitedFrameCodec,
    /// Watch signalled from the decoder.
    frames: Arc<FrameWatch>,
}

impl ObservingCodec {
    /// Wrap the client's frame codec in a frame watch.
    fn new(frames: Arc<FrameWatch>) -> Self {
        Self {
            inner: client_codec(),
            frames,
        }
    }
}

impl FrameCodec for ObservingCodec {
    type Frame = Bytes;
    type Decoder = ObservingDecoder;
    type Encoder = <LengthDelimitedFrameCodec as FrameCodec>::Encoder;

    fn decoder(&self) -> Self::Decoder {
        ObservingDecoder {
            inner: self.inner.decoder(),
            frames: Arc::clone(&self.frames),
        }
    }

    fn encoder(&self) -> Self::Encoder { self.inner.encoder() }

    fn frame_payload(frame: &Self::Frame) -> &[u8] {
        LengthDelimitedFrameCodec::frame_payload(frame)
    }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes {
        LengthDelimitedFrameCodec::frame_payload_bytes(frame)
    }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame { self.inner.wrap_payload(payload) }

    fn max_frame_length(&self) -> usize { self.inner.max_frame_length() }
}

/// Decoder half of [`ObservingCodec`].
struct ObservingDecoder {
    /// Decoder that owns the framing state.
    inner: LengthDelimitedDecoder,
    /// Watch signalled after each complete frame.
    frames: Arc<FrameWatch>,
}

impl Decoder for ObservingDecoder {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let frame = self.inner.decode(src)?;
        if frame.is_some() {
            self.frames.frame_decoded();
        }
        Ok(frame)
    }

    /// Forwarding this is what keeps the truncation contract intact: the
    /// bundled decoder is the half that classifies a partial frame.
    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let frame = self.inner.decode_eof(src)?;
        if frame.is_some() {
            self.frames.frame_decoded();
        }
        Ok(frame)
    }
}

/// Build the application with an observing codec and the recording protocol.
///
/// Everything but the codec matches [`build_app`], so this scenario differs
/// from the other hook tests only in how it observes the read half.
fn build_observing_app(
    log: &Arc<HookLog>,
    ended: oneshot::Sender<()>,
    handler: ResponseHandler<Envelope>,
    frames: Arc<FrameWatch>,
) -> TestResult<WireframeApp<BincodeSerializer, (), Envelope, ObservingCodec>> {
    Ok(WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route_response(ROUTE, handler)?
        .with_codec(ObservingCodec::new(frames))
        .with_protocol(RecordingProtocol::new(Arc::clone(log), None))
        .on_connection_setup(|| async {})?
        .on_connection_teardown(ended_notifier(ended))?)
}

/// Handler that announces its start, stays pending, then fails its response.
///
/// A failed response ends dispatch, which is the early stop this scenario
/// needs. Holding the response open gives the connection the chance to buffer
/// the request pipelined behind it.
fn failing_after(started: Arc<Notify>, release: Arc<Notify>) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let handler_started = Arc::clone(&started);
        let gate = Arc::clone(&release);
        Box::pin(async move {
            handler_started.notify_one();
            gate.notified().await;
            let stream: FrameStream<Envelope, ()> =
                Box::pin(stream::iter(vec![Err(WireframeError::Protocol(()))]));
            Response::Stream(stream)
        })
    })
}
