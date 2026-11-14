//! Fixtures shared by the connection-lifecycle matrix.
//!
//! The support code here builds a setup value whose identity and drop are
//! observable, scripts the transports that drive each terminal outcome, and
//! asserts that teardown received exactly the value setup created.

use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::Notify,
};
pub(super) use wireframe::test_helpers::{UnitFuture, counting_teardown};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    message_assembler::{MessageAssembler, ParsedFrameHeader},
    serializer::BincodeSerializer,
    test_helpers::TestAssembler,
};
use wireframe_testing::TestResult;

use super::{build_frame, fallible_check_equal::check_equal};

/// Consecutive malformed envelopes needed to trip the decode-failure limit.
pub(super) const DECODE_FAILURE_LIMIT: usize = 10;
/// One-completion frame whose payload cannot deserialize as an envelope.
const MALFORMED_FRAME: [u8; 5] = [0, 0, 0, 1, 0xff];
/// Truncated frame that ends processing with an unexpected-EOF error.
const TRUNCATED_FRAME: [u8; 5] = [0, 0, 0, 2, 1];

/// Application type whose connection state is an identity-tracking token.
type LifecycleApp = WireframeApp<BincodeSerializer, OwnedToken, Envelope>;

/// Terminal outcome exercised by one matrix row.
#[derive(Clone, Copy, Debug)]
pub(super) enum TerminalOutcome {
    /// The peer closes the stream without sending a frame.
    CleanEof,
    /// A truncated frame fails to decode.
    DecodeError,
    /// Repeated malformed envelopes exhaust the decode-failure limit.
    ProtocolError,
    /// The transport read itself fails.
    ReadError,
    /// A response write fails.
    WriteError,
    /// A route handler panics.
    HandlerPanic,
}

/// Observation log shared by setup, teardown, and token drops.
#[derive(Clone, Debug, Default)]
struct TokenLog {
    created: Vec<u64>,
    receipts: Vec<u64>,
    dropped: Vec<u64>,
}

/// Shared observation log behind a poison-tolerant mutex.
#[derive(Clone, Default)]
pub(super) struct Recorder(Arc<Mutex<TokenLog>>);

impl Recorder {
    /// Run `observe` against the log, recovering from a poisoned mutex.
    ///
    /// A panic while a lock is held poisons it, and this fixture exists to
    /// observe panicking connections, so poisoning must not mask the result.
    fn observe<R>(&self, observe: impl FnOnce(&mut TokenLog) -> R) -> R {
        let mut log = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        observe(&mut log)
    }

    /// Record that setup created a token.
    fn record_created(&self, id: u64) { self.observe(|log| log.created.push(id)); }

    /// Record that teardown received a token.
    fn record_receipt(&self, id: u64) { self.observe(|log| log.receipts.push(id)); }

    /// Record that a token was dropped.
    fn record_drop(&self, id: u64) { self.observe(|log| log.dropped.push(id)); }

    /// Return a snapshot of the log.
    fn snapshot(&self) -> TokenLog { self.observe(|log| log.clone()) }
}

/// Non-`Clone`, non-`Copy` setup value whose identity is observable.
pub(super) struct OwnedToken {
    id: u64,
    recorder: Recorder,
}

impl OwnedToken {
    /// Create a token and record its identity.
    fn create(recorder: Recorder, id: u64) -> Self {
        recorder.record_created(id);
        Self { id, recorder }
    }
}

impl Drop for OwnedToken {
    fn drop(&mut self) { self.recorder.record_drop(self.id); }
}

/// How the scripted transport behaves on its first read.
#[derive(Clone, Copy, Debug)]
enum ReadMode {
    /// Serve the scripted input, then report end of stream.
    Scripted,
    /// Fail immediately with a connection-reset error.
    Fail,
}

/// How the scripted transport behaves on writes.
#[derive(Clone, Copy, Debug)]
enum WriteMode {
    /// Accept and discard written bytes.
    Sink,
    /// Fail immediately with a broken-pipe error.
    Fail,
}

/// In-memory transport that replays scripted bytes and can fail one direction.
pub(super) struct ScriptedStream {
    input: Vec<u8>,
    pos: usize,
    read_mode: ReadMode,
    write_mode: WriteMode,
}

impl ScriptedStream {
    /// Serve `input`, then report end of stream.
    pub(super) fn new(input: Vec<u8>) -> Self {
        Self {
            input,
            pos: 0,
            read_mode: ReadMode::Scripted,
            write_mode: WriteMode::Sink,
        }
    }

    /// Report a transport read failure before serving any bytes.
    fn failing_read() -> Self {
        Self {
            input: Vec::new(),
            pos: 0,
            read_mode: ReadMode::Fail,
            write_mode: WriteMode::Sink,
        }
    }

    /// Serve `input`, then fail the first response write.
    fn failing_write(input: Vec<u8>) -> Self {
        Self {
            input,
            pos: 0,
            read_mode: ReadMode::Scripted,
            write_mode: WriteMode::Fail,
        }
    }
}

impl AsyncRead for ScriptedStream {
    /// Replay scripted bytes and signal EOF once exhausted, or fail outright.
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if matches!(this.read_mode, ReadMode::Fail) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "scripted read failure",
            )));
        }
        let remaining = this.input.get(this.pos..).unwrap_or_default();
        let take = remaining.len().min(buf.remaining());
        if let Some(chunk) = remaining.get(..take) {
            buf.put_slice(chunk);
        }
        this.pos += take;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for ScriptedStream {
    /// Discard written bytes, or fail immediately according to the script.
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if matches!(this.write_mode, WriteMode::Fail) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "scripted write failure",
            )));
        }
        Poll::Ready(Ok(buf.len()))
    }

    /// Report a successful flush without buffering.
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    /// Report a successful shutdown; the scripted stream owns no resources.
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Build a lifecycle application around `handler`.
pub(super) fn lifecycle_app(
    recorder: Recorder,
    next_id: Arc<AtomicU64>,
    handler: Handler<Envelope>,
) -> TestResult<LifecycleApp> {
    build_lifecycle_app(recorder, next_id, handler, None)
}

/// Test assembler that counts the frame headers it rejects.
pub(super) struct RejectingAssembler {
    /// Number of headers the shared test assembler refused.
    rejected: Arc<AtomicUsize>,
}

impl RejectingAssembler {
    /// Wrap the shared test assembler and count rejected headers.
    pub(super) const fn new(rejected: Arc<AtomicUsize>) -> Self { Self { rejected } }
}

impl MessageAssembler for RejectingAssembler {
    fn parse_frame_header(&self, payload: &[u8]) -> io::Result<ParsedFrameHeader> {
        let result = TestAssembler.parse_frame_header(payload);
        if result.is_err() {
            self.rejected.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
}

/// Build a lifecycle application that also runs message assembly.
///
/// With an assembler installed, a decoded envelope payload is treated as an
/// assembly frame header. This lets the matrix exercise a protocol failure
/// raised after envelope decoding rather than during it. `rejected` counts
/// headers the assembler refused, proving the failure came from assembly.
pub(super) fn assembling_lifecycle_app(
    recorder: Recorder,
    next_id: Arc<AtomicU64>,
    handler: Handler<Envelope>,
    rejected: Arc<AtomicUsize>,
) -> TestResult<LifecycleApp> {
    build_lifecycle_app(recorder, next_id, handler, Some(rejected))
}

/// Assemble the lifecycle application, optionally installing the test
/// assembler.
fn build_lifecycle_app(
    recorder: Recorder,
    next_id: Arc<AtomicU64>,
    handler: Handler<Envelope>,
    rejected: Option<Arc<AtomicUsize>>,
) -> TestResult<LifecycleApp> {
    let base_app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?.route(1, handler)?;
    let app = match rejected {
        Some(counter) => base_app.with_message_assembler(RejectingAssembler::new(counter)),
        None => base_app,
    };
    let setup_recorder = recorder.clone();
    let teardown_recorder = recorder;
    Ok(app
        .on_connection_setup(move || {
            let owner = setup_recorder.clone();
            let id = next_id.fetch_add(1, Ordering::SeqCst);
            async move { OwnedToken::create(owner, id) }
        })?
        .on_connection_teardown(move |token: OwnedToken| {
            let observer = teardown_recorder.clone();
            async move {
                let id = token.id;
                observer.record_receipt(id);
                drop(token);
            }
        })?)
}

/// Build the scripted transport for one terminal outcome.
pub(super) fn stream_for(outcome: TerminalOutcome) -> TestResult<ScriptedStream> {
    let stream = match outcome {
        TerminalOutcome::CleanEof => ScriptedStream::new(Vec::new()),
        TerminalOutcome::DecodeError => ScriptedStream::new(TRUNCATED_FRAME.to_vec()),
        TerminalOutcome::ProtocolError => {
            let mut input = Vec::new();
            for _ in 0..DECODE_FAILURE_LIMIT {
                input.extend_from_slice(&MALFORMED_FRAME);
            }
            ScriptedStream::new(input)
        }
        TerminalOutcome::ReadError => ScriptedStream::failing_read(),
        TerminalOutcome::WriteError => ScriptedStream::failing_write(build_frame(1, vec![b'X'])?),
        TerminalOutcome::HandlerPanic => ScriptedStream::new(build_frame(1, vec![b'X'])?),
    };
    Ok(stream)
}

/// Route handler that panics when invoked.
pub(super) fn panicking_handler() -> Handler<Envelope> {
    Arc::new(|_: &Envelope| Box::pin(async { panic!("handler boom") }))
}

/// Setup hook that panics before producing connection state.
pub(super) fn panicking_setup() -> impl Fn() -> UnitFuture + Send + Sync + 'static {
    || Box::pin(async { panic!("setup boom") })
}

/// Setup hook that waits on `gate` and therefore never completes.
pub(super) fn gated_setup(gate: Arc<Notify>) -> impl Fn() -> UnitFuture + Send + Sync + 'static {
    move || {
        let pending_gate = Arc::clone(&gate);
        Box::pin(async move {
            pending_gate.notified().await;
        })
    }
}

/// Teardown callback that panics on its first invocation and succeeds after.
pub(super) fn panicking_teardown(
    invocations: Arc<AtomicUsize>,
) -> impl Fn(()) -> UnitFuture + Send + Sync + 'static {
    move |()| {
        let counter = Arc::clone(&invocations);
        Box::pin(async move {
            assert!(counter.fetch_add(1, Ordering::SeqCst) != 0, "teardown boom");
        })
    }
}

/// Assert the token was created once, received once, and dropped once.
pub(super) fn assert_single_lifecycle(recorder: &Recorder) -> TestResult<()> {
    let log = recorder.snapshot();
    check_equal(
        &log.created.len(),
        &1usize,
        "setup should create exactly one token",
    )?;
    check_equal(
        &log.receipts.len(),
        &1usize,
        "teardown should receive exactly one token",
    )?;
    check_equal(
        &log.dropped.len(),
        &1usize,
        "the token should be dropped exactly once",
    )?;
    let received = log
        .receipts
        .first()
        .ok_or("teardown should receive a token")?;
    let created = log.created.first().ok_or("setup should create a token")?;
    check_equal(
        received,
        created,
        "teardown should receive the exact setup value",
    )?;
    Ok(())
}
