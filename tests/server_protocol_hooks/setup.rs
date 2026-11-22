//! Setup ordering: a push before any request, and setup before the first read.
//!
//! `on_connection_setup` runs before the connection reads anything, so a
//! protocol that queues a push there has it written without waiting for an
//! inbound frame. Two independent observations pin that ordering: the peer
//! receives the push with nothing sent, and the connection's decoder records
//! whether it ran before setup rather than relying on log order alone.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use bytes::{Bytes, BytesMut};
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_util::codec::{Decoder, Framed};
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::{FrameCodec, LengthDelimitedEncoder, LengthDelimitedFrameCodec},
    hooks::{ConnectionContext, WireframeProtocol},
    push::{PushHandle, PushPolicy, PushPriority},
    serializer::BincodeSerializer,
};
use wireframe_testing::TestResult;

use super::harness::{
    ROUTE,
    await_connection_end,
    build_request,
    client_codec,
    decode_unmarked_envelope,
    ended_notifier,
    ensure,
    next_frame,
    single_handler,
    start_server,
};

/// Payload of the push the protocol queues from `on_connection_setup`.
const EARLY_PUSH: &[u8] = b"EARLY";
/// Maximum frame length the probe codec accepts.
const MAX_FRAME_LENGTH: usize = 2048;

/// Cross-checks between the protocol and the connection's decoder.
#[derive(Default)]
struct SetupProbe {
    /// Set by `on_connection_setup`.
    setup_done: AtomicBool,
    /// Set if a decode ran before setup completed.
    read_before_setup: AtomicBool,
    /// Set if the setup push was refused a queue slot.
    push_refused: AtomicBool,
}

impl SetupProbe {
    /// Record that the protocol's setup hook is running.
    fn mark_setup(&self) { self.setup_done.store(true, Ordering::Release); }

    /// Record that the setup push was refused.
    fn note_refused_push(&self) { self.push_refused.store(true, Ordering::Release); }

    /// Note whether this decode ran before setup completed.
    fn observe_decode(&self) {
        if !self.setup_done.load(Ordering::Acquire) {
            self.read_before_setup.store(true, Ordering::Release);
        }
    }

    /// Whether any decode ran before setup completed.
    fn read_before_setup(&self) -> bool { self.read_before_setup.load(Ordering::Acquire) }

    /// Whether the setup push was refused.
    fn push_refused(&self) -> bool { self.push_refused.load(Ordering::Acquire) }
}

/// Codec that records decodes against the setup probe.
#[derive(Clone)]
struct ProbeCodec {
    /// Probe shared with the protocol.
    probe: Arc<SetupProbe>,
}

impl FrameCodec for ProbeCodec {
    type Frame = Bytes;
    type Decoder = ProbeDecoder;
    type Encoder = LengthDelimitedEncoder;

    fn decoder(&self) -> Self::Decoder {
        ProbeDecoder {
            probe: Arc::clone(&self.probe),
            inner: LengthDelimitedFrameCodec::default().decoder(),
        }
    }

    fn encoder(&self) -> Self::Encoder { LengthDelimitedFrameCodec::default().encoder() }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.as_ref() }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Bytes { payload }

    fn max_frame_length(&self) -> usize { MAX_FRAME_LENGTH }
}

/// Length-delimited decoder that reports each decode to the probe.
struct ProbeDecoder {
    /// Probe shared with the protocol.
    probe: Arc<SetupProbe>,
    /// Delegate that performs the real framing.
    inner: <LengthDelimitedFrameCodec as FrameCodec>::Decoder,
}

impl Decoder for ProbeDecoder {
    type Item = Bytes;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.probe.observe_decode();
        self.inner.decode(src)
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.probe.observe_decode();
        self.inner.decode_eof(src)
    }
}

/// Protocol that marks setup and optionally pushes before anything is read.
struct EarlyPushProtocol {
    /// Probe shared with the codec.
    probe: Arc<SetupProbe>,
    /// Frame queued from `on_connection_setup`, if any.
    push: Option<Bytes>,
}

impl WireframeProtocol for EarlyPushProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn on_connection_setup(&self, handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {
        self.probe.mark_setup();
        let Some(frame) = self.push.clone() else {
            return;
        };
        if handle
            .try_push(frame, PushPriority::High, PushPolicy::ReturnErrorIfFull)
            .is_err()
        {
            self.probe.note_refused_push();
        }
    }
}

/// Build an app whose protocol pushes from setup and whose decoder is probed.
fn probed_app(
    probe: &Arc<SetupProbe>,
    push: Option<Bytes>,
    ended: tokio::sync::oneshot::Sender<()>,
) -> TestResult<WireframeApp<BincodeSerializer, (), Envelope, ProbeCodec>> {
    Ok(WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .with_codec(ProbeCodec {
            probe: Arc::clone(probe),
        })
        .route_response(ROUTE, single_handler())?
        .with_protocol(EarlyPushProtocol {
            probe: Arc::clone(probe),
            push,
        })
        .on_connection_setup(|| async {})?
        .on_connection_teardown(ended_notifier(ended))?)
}

/// A push queued during setup reaches a peer that sent no request at all.
#[tokio::test]
async fn a_push_queued_at_setup_reaches_a_silent_peer() -> TestResult {
    let probe = Arc::new(SetupProbe::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    // Pushes carry the frame payload, which the connection encodes for the
    // wire; encoding it here would nest a second length prefix.
    let push_frame = Bytes::from_static(EARLY_PUSH);
    let app = probed_app(&probe, Some(push_frame), ended_tx)?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    // The default unit preamble still reads one byte before the application
    // connection starts. That byte is rewound into the stream, where it sits as
    // an incomplete length prefix: it completes no frame, so the push below
    // still precedes every inbound request.
    client.write_all(&[0]).await?;
    let mut frames = Framed::new(client, client_codec().decoder());
    let pushed = next_frame(&mut frames).await?;
    ensure(
        pushed.as_ref() == EARLY_PUSH,
        format!("the setup push must reach a peer that sent nothing: {pushed:?}"),
    )?;
    ensure(
        !probe.push_refused(),
        "the setup push must be accepted by the connection's queue",
    )?;

    drop(frames);
    await_connection_end(ended_rx).await?;
    server.stop().await
}

/// The protocol's setup hook runs before the connection decodes anything.
///
/// The decoder records each decode against the probe, so this fails if setup
/// moves after the first read even when the logged event order looks the same.
#[tokio::test]
async fn setup_runs_before_the_first_read() -> TestResult {
    let probe = Arc::new(SetupProbe::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    let app = probed_app(&probe, None, ended_tx)?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    client.write_all(&build_request(ROUTE)?).await?;
    let mut frames = Framed::new(client, client_codec().decoder());
    let reply = decode_unmarked_envelope(&next_frame(&mut frames).await?)?;
    ensure(reply.payload_bytes() == [9], "the handler must reply")?;

    drop(frames);
    await_connection_end(ended_rx).await?;
    ensure(
        !probe.read_before_setup(),
        "the connection must run setup before it decodes an inbound frame",
    )?;

    server.stop().await
}
