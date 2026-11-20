//! End-to-end recovery behaviour for decoder failures on `WireframeServer`.
//!
//! Each case drives a real TCP connection through the server supervisor, sends
//! a malformed frame, and checks both the observable connection behaviour and
//! `wireframe_codec_errors_total`. Metrics use a current-thread runtime because
//! the local recorder must stay installed while the server's connection tasks
//! emit their counts.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{future::Future, io, net::SocketAddr, sync::Arc, time::Duration};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use metrics::with_local_recorder;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    runtime::Builder,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{Instant, timeout},
};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    codec::{
        CodecError,
        CodecErrorContext,
        EofError,
        FrameCodec,
        FramingError,
        LengthDelimitedFrameCodec,
        ProtocolError,
        RecoveryConfig,
        RecoveryPolicy,
        RecoveryPolicyHook,
    },
    serializer::{BincodeSerializer, Serializer},
    server::{ServerError, WireframeServer},
};
use wireframe_testing::{
    ObservabilityHandle,
    TestResult,
    unused_listener,
    wait_for_server_readiness,
};

/// First payload byte that marks a frame as malformed.
const MALFORMED: u8 = 0xff;
/// Routed message identifier handled by the test application.
const MESSAGE_ID: u32 = 1;
/// Payload carried by the valid frame that follows a malformed one.
const VALID_PAYLOAD: &[u8] = b"ok";
/// Quarantine pause used by the quarantine case.
const QUARANTINE: Duration = Duration::from_millis(100);
/// Upper bound for any single network step; a bound, not an expected delay.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// Observation recorded by [`RecordingHook::on_error`].
struct RecoveryEvent {
    /// Error category the connection loop classified the failure as.
    error_type: &'static str,
    /// Policy the connection loop actually applied.
    policy: RecoveryPolicy,
    /// Moment the callback ran, used to measure the quarantine pause.
    at: Instant,
    /// Peer address the connection loop forwarded to the hook.
    peer: Option<SocketAddr>,
}

/// Hook with a fixed policy that reports every callback through a channel.
struct RecordingHook {
    /// Policy returned for every error.
    selected: RecoveryPolicy,
    /// Duration returned for a quarantine.
    quarantine: Duration,
    /// Sink for observed callbacks.
    events: mpsc::UnboundedSender<RecoveryEvent>,
}

impl RecoveryPolicyHook for RecordingHook {
    fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        self.selected
    }

    fn quarantine_duration(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> Duration {
        self.quarantine
    }

    fn on_error(&self, error: &CodecError, ctx: &CodecErrorContext, policy: RecoveryPolicy) {
        // The receiver only exists to observe the hook, so a closed channel is
        // not an error worth reporting from a test hook.
        drop(self.events.send(RecoveryEvent {
            error_type: error.error_type(),
            policy,
            at: Instant::now(),
            peer: ctx.peer_address,
        }));
    }
}

/// Application type served by every case in this file.
type TestApp = WireframeApp<BincodeSerializer, (), Envelope, TestCodec>;

/// Application type served by the built-in-codec cases.
type BuiltInApp = WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>;

/// Largest payload accepted by the built-in codec in the oversize case.
const BUILT_IN_MAX_FRAME_LENGTH: usize = 256;

/// Handler that leaves the request envelope untouched so the reply echoes it.
fn echo_handler() -> Handler<Envelope> { Arc::new(|_: &Envelope| Box::pin(async {})) }

/// A running server plus the handles needed to stop it.
struct RunningServer {
    /// Bound address clients connect to.
    address: SocketAddr,
    /// Signal that starts a graceful shutdown.
    shutdown: oneshot::Sender<()>,
    /// Supervisor task, awaited after shutdown.
    task: JoinHandle<Result<(), ServerError>>,
}

/// Start a one-worker server for `app` and wait until it accepts connections.
async fn start_server<Codec>(
    app: WireframeApp<BincodeSerializer, (), Envelope, Codec>,
) -> TestResult<RunningServer>
where
    Codec: FrameCodec,
{
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(unused_listener()?)?;
    let address = server
        .local_addr()
        .ok_or_else(|| "server did not report a bound address".to_owned())?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                drop(shutdown_rx.await);
            })
            .await
    });
    wait_for_server_readiness(ready_rx).await?;
    Ok(RunningServer {
        address,
        shutdown: shutdown_tx,
        task,
    })
}

impl RunningServer {
    /// Ask the supervisor to stop and await its result.
    async fn stop(self) -> TestResult {
        self.shutdown
            .send(())
            .map_err(|()| "server shutdown receiver was dropped")?;
        timeout(STEP_TIMEOUT, self.task)
            .await
            .map_err(|_| "server did not shut down")???;
        Ok(())
    }
}

/// Build the application under test with an optional hook and drop budget.
fn test_app(hook: Option<RecordingHook>, max_consecutive_drops: u32) -> TestResult<TestApp> {
    // `new` requires a default-constructible codec, so the default application
    // is built first and the test codec installed afterwards.
    let base: WireframeApp = WireframeApp::new()?;
    let mut app = base
        .with_codec(TestCodec { max: 1024 })
        .recovery_config(RecoveryConfig::default().max_consecutive_drops(max_consecutive_drops))
        .route(MESSAGE_ID, echo_handler())?;
    if let Some(recording) = hook {
        app = app.with_recovery_hook(recording);
    }
    Ok(app)
}

/// Build the application under test with the built-in length-delimited codec.
fn built_in_app(hook: RecordingHook) -> TestResult<BuiltInApp> {
    // `new` requires a default-constructible codec, so the default application
    // is built first and the built-in codec installed afterwards.
    let base: WireframeApp = WireframeApp::new()?;
    Ok(base
        .with_codec(LengthDelimitedFrameCodec::new(BUILT_IN_MAX_FRAME_LENGTH))
        .route(MESSAGE_ID, echo_handler())?
        .with_recovery_hook(hook))
}

/// Read one length-prefixed reply payload from the client connection.
async fn read_reply(stream: &mut TcpStream) -> TestResult<Vec<u8>> {
    let mut header = [0_u8; 4];
    timeout(STEP_TIMEOUT, stream.read_exact(&mut header))
        .await
        .map_err(|_| "timed out reading the reply header")??;
    let mut header_slice: &[u8] = &header;
    let length = header_slice.get_u32() as usize;
    let mut payload = vec![0_u8; length];
    timeout(STEP_TIMEOUT, stream.read_exact(&mut payload))
        .await
        .map_err(|_| "timed out reading the reply payload")??;
    Ok(payload)
}

/// Assert the server closed the connection instead of sending another frame.
async fn expect_closed(stream: &mut TcpStream) -> TestResult {
    let mut byte = [0_u8; 1];
    match timeout(STEP_TIMEOUT, stream.read(&mut byte)).await {
        Ok(Ok(0)) => Ok(()),
        Ok(Ok(_)) => Err("server sent a frame after a terminal failure".into()),
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionReset => Ok(()),
        Ok(Err(error)) => Err(format!("unexpected read error: {error}").into()),
        Err(_) => Err("server left the connection open after a terminal failure".into()),
    }
}

/// Run one case on a current-thread runtime with the local metrics recorder.
fn capture<F>(case: F) -> TestResult<ObservabilityHandle>
where
    F: Future<Output = TestResult>,
{
    let mut observability = ObservabilityHandle::new();
    let runtime = Builder::new_current_thread().enable_all().build()?;
    with_local_recorder(observability.recorder(), || runtime.block_on(case))?;
    observability.snapshot();
    Ok(observability)
}

/// Collect whatever the hook reported before the server stopped.
fn drain_events(events: &mut mpsc::UnboundedReceiver<RecoveryEvent>) -> Vec<RecoveryEvent> {
    let mut collected = Vec::new();
    while let Ok(event) = events.try_recv() {
        collected.push(event);
    }
    collected
}

/// Assert the recorder saw exactly the expected codec-error series.
///
/// Comparing the whole label set, rather than probing the expected pairs,
/// rejects an extra series that a misclassification would create — for
/// example an `error_type="io"` count recorded beside the expected
/// `"framing"` one.
fn assert_only_codec_errors(
    observability: &ObservabilityHandle,
    expected: &[(&str, &str, u64)],
) -> TestResult {
    let mut actual = observability.codec_error_series();
    actual.sort();
    let mut wanted: Vec<(String, String, u64)> = expected
        .iter()
        .map(|&(error_type, policy, count)| (error_type.to_owned(), policy.to_owned(), count))
        .collect();
    wanted.sort();
    if actual == wanted {
        Ok(())
    } else {
        Err(format!("codec error series were {actual:?}, expected {wanted:?}").into())
    }
}

#[path = "codec_recovery_server/test_codec.rs"]
mod test_codec;

use self::test_codec::{
    TestCodec,
    envelope_frame,
    malformed_frame,
    oversized_frame,
    truncated_frame,
};

#[path = "codec_recovery_server/cases.rs"]
mod cases;
