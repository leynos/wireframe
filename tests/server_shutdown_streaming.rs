//! A server stop must not truncate a response its peer is still reading.
//!
//! ADR-014 requires graceful drain: a connection in flight across `stop()` keeps
//! running until its work completes. For a streamed response that means every
//! frame, the protocol terminator, and the completion hook must still be
//! delivered after the stop request, because only a write that cannot make
//! progress is abandoned.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::oneshot,
    time::{Instant, timeout},
};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    hooks::{ConnectionContext, WireframeProtocol},
    response::{FrameStream, Response},
    serializer::{BincodeSerializer, Serializer},
    server::WireframeServer,
};
use wireframe_testing::{
    TestError,
    TestResult,
    encode_frame,
    unused_listener,
    wait_for_server_readiness,
};

/// Application type shared by the request builder and the server under test.
type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Route whose producer paces each frame so a stop lands mid-response.
const STREAM_ROUTE: u32 = 60;
/// Frames the paced producer sends.
const FRAME_COUNT: u8 = 3;
/// Pause before each frame: long enough that the whole response outlasts the
/// client observing the first frame, far below the thirty-second idle grace.
const FRAME_PAUSE: Duration = Duration::from_millis(50);
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Payload of the protocol terminator frame.
const TERMINATOR: &[u8] = b"END";
/// Upper bound for a response frame that must already be in flight.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound for the connection to drain after the client closes.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Require `condition`, reporting `message` as the test failure otherwise.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Protocol that terminates a streamed response and counts completion hooks.
struct TerminatingProtocol {
    /// Number of `on_command_end` invocations.
    completions: Arc<AtomicUsize>,
}

impl WireframeProtocol for TerminatingProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        // The frame is written unchanged, and the transport's codec frames it
        // like any other payload.
        Some(Bytes::from_static(TERMINATOR))
    }

    fn on_command_end(&self, _ctx: &mut ConnectionContext) {
        self.completions.fetch_add(1, Ordering::SeqCst);
    }
}

/// Handle a request by streaming numbered frames with a pause between them.
fn paced_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let stream: FrameStream<Envelope, ()> = Box::pin(async_stream::try_stream! {
                for payload in 1..=FRAME_COUNT {
                    tokio::time::sleep(FRAME_PAUSE).await;
                    yield Envelope::new(STREAM_ROUTE, None, vec![payload]);
                }
            });
            Response::Stream(stream)
        })
    })
}

/// Encode an empty request for `route` in the transport's frame format.
fn build_request(route: u32) -> TestResult<Vec<u8>> {
    let encoded = BincodeSerializer.serialize(&Envelope::new(route, Some(7), Vec::new()))?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, encoded)?)
}

/// Decode one response frame and return its payload.
fn decode_payload(bytes: &[u8]) -> TestResult<Vec<u8>> {
    let (envelope, consumed) = BincodeSerializer
        .deserialize::<Envelope>(bytes)
        .map_err(|error| TestError::Msg(format!("deserialize frame: {error}")))?;
    ensure(
        consumed == bytes.len(),
        format!(
            "trailing bytes after envelope: consumed {consumed} of {}",
            bytes.len()
        ),
    )?;
    Ok(envelope.payload_bytes().to_vec())
}

/// Read the next frame, failing the test if none arrives in time.
async fn next_frame<S>(frames: &mut S) -> TestResult<Bytes>
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    timeout(FRAME_TIMEOUT, frames.next())
        .await
        .map_err(|_| TestError::Msg("the response stalled while the peer was reading".into()))?
        .transpose()
        .map_err(|error| TestError::Msg(format!("read response frame: {error}")))?
        .ok_or_else(|| TestError::Msg("the response ended before its terminator".into()))
}

/// Wait until the completion hook has run, or fail after `DRAIN_TIMEOUT`.
async fn await_completion(completions: &AtomicUsize) -> TestResult {
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    while completions.load(Ordering::SeqCst) == 0 {
        if Instant::now() >= deadline {
            return Err("the completion hook never ran for the drained response".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

/// A stop during a readable stream still delivers every frame and the hook.
#[tokio::test]
async fn stop_drains_a_stream_the_peer_is_still_reading() -> TestResult {
    let completions = Arc::new(AtomicUsize::new(0));
    let app = TestApp::default()
        .route_response(STREAM_ROUTE, paced_handler())?
        .with_protocol(TerminatingProtocol {
            completions: Arc::clone(&completions),
        });
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let shutdown = server.ready_signal(ready_tx).spawn().await?;
    wait_for_server_readiness(ready_rx).await?;

    let mut client = TcpStream::connect(addr).await?;
    client.write_all(&build_request(STREAM_ROUTE)?).await?;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut frames = Framed::new(client, codec.decoder());

    let first = next_frame(&mut frames).await?;
    ensure(
        decode_payload(&first)? == vec![1],
        "the first frame must arrive before the stop request",
    )?;

    // The server is now draining, with two frames and the terminator still to
    // come. A drain that abandoned the response would end the stream here.
    shutdown.stop();

    for expected in 2..=FRAME_COUNT {
        let payload = decode_payload(&next_frame(&mut frames).await?)?;
        ensure(
            payload == vec![expected],
            format!("frame {expected} must survive the stop request: {payload:?}"),
        )?;
    }
    let terminator = next_frame(&mut frames).await?;
    ensure(
        terminator.as_ref() == TERMINATOR,
        format!("the drained response must end with its protocol terminator: {terminator:?}"),
    )?;

    await_completion(&completions).await?;
    ensure(
        completions.load(Ordering::SeqCst) == 1,
        "the completion hook must run exactly once for the drained response",
    )?;

    drop(frames);
    timeout(DRAIN_TIMEOUT, shutdown.drained())
        .await
        .map_err(|_| TestError::Msg("the server did not drain after the peer closed".into()))??;
    Ok(())
}
