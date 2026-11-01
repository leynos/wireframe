//! Teardown regression coverage for returned stream-processing errors.
//!
//! Each scenario drives a prepared application far enough for setup to produce
//! state, then terminates the connection through a decode or send failure. The
//! teardown hook must still run exactly once, receive that same state, and run
//! after the transport activity that drove stream processing.

use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use wireframe::{
    app::{Envelope, PreparedApp, WireframeApp},
    serializer::BincodeSerializer,
};
use wireframe_testing::TestResult;

use super::{
    build_frame,
    fallible_check::check,
    fallible_check_equal::check_equal,
    handler,
    response_payload,
};

/// Non-unit sentinel that must survive from setup into teardown.
const SENTINEL_STATE: u32 = 0x00c0_ffee;
/// Consecutive malformed envelopes needed to trip the decode-failure limit.
const DECODE_FAILURE_LIMIT: usize = 10;
/// One-completion frame whose payload cannot deserialize as an envelope.
const MALFORMED_FRAME: [u8; 5] = [0, 0, 0, 1, 0xff];

/// Application type whose lifecycle state is the non-unit sentinel.
type LifecycleApp = WireframeApp<BincodeSerializer, u32, Envelope>;
/// Prepared form of [`LifecycleApp`].
type LifecyclePreparedApp = PreparedApp<BincodeSerializer, u32, Envelope>;

/// Ordered event recorded while a scripted connection runs.
///
/// The order proves that teardown observes stream-processing work rather than
/// running before the connection is driven.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleEvent {
    /// The transport served bytes, or end of stream, to the frame pipeline.
    TransportRead,
    /// The frame pipeline attempted to write a response.
    TransportWrite,
    /// The teardown hook started.
    Teardown,
}

/// Lock `mutex`, recovering the guard if an earlier holder panicked.
///
/// Ordering observations must survive a panic long enough to describe it, and
/// recovering avoids panicking a second time from a `Drop` or poll path.
fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Shared observations recorded by the lifecycle hooks under test.
#[derive(Clone)]
struct LifecycleProbe {
    setup_calls: Arc<AtomicUsize>,
    teardown_calls: Arc<AtomicUsize>,
    teardown_state: Arc<AtomicU32>,
    events: Arc<Mutex<Vec<LifecycleEvent>>>,
}

impl LifecycleProbe {
    /// Create counters and an event log that start empty.
    fn new() -> Self {
        Self {
            setup_calls: Arc::new(AtomicUsize::new(0)),
            teardown_calls: Arc::new(AtomicUsize::new(0)),
            teardown_state: Arc::new(AtomicU32::new(0)),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Build an application whose setup returns the sentinel and whose
    /// teardown records its call count, the state it receives, and its
    /// position in the ordered event log.
    fn app(&self) -> TestResult<LifecycleApp> {
        let setup_calls = Arc::clone(&self.setup_calls);
        let teardown_calls = Arc::clone(&self.teardown_calls);
        let teardown_state = Arc::clone(&self.teardown_state);
        let events = Arc::clone(&self.events);

        Ok(LifecycleApp::new()?
            .route(1, handler())?
            .on_connection_setup(move || {
                let setup_calls = Arc::clone(&setup_calls);
                async move {
                    setup_calls.fetch_add(1, Ordering::SeqCst);
                    SENTINEL_STATE
                }
            })?
            .on_connection_teardown(move |state: u32| {
                let teardown_calls = Arc::clone(&teardown_calls);
                let teardown_state = Arc::clone(&teardown_state);
                let events = Arc::clone(&events);
                async move {
                    lock_recover(&events).push(LifecycleEvent::Teardown);
                    teardown_state.store(state, Ordering::SeqCst);
                    teardown_calls.fetch_add(1, Ordering::SeqCst);
                }
            })?)
    }

    /// Prepare the probe application for connection-level driving.
    async fn prepare(&self) -> TestResult<LifecyclePreparedApp> {
        let prepared = self
            .app()?
            .prepare()
            .await
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
        Ok(prepared)
    }

    /// Snapshot the ordered events recorded so far.
    fn events(&self) -> Vec<LifecycleEvent> { lock_recover(&self.events).clone() }

    /// Assert setup and teardown each ran once and the sentinel arrived.
    fn assert_exactly_once(&self) -> TestResult<()> {
        check_equal(
            &self.setup_calls.load(Ordering::SeqCst),
            &1usize,
            "setup callback should run exactly once",
        )?;
        check_equal(
            &self.teardown_calls.load(Ordering::SeqCst),
            &1usize,
            "teardown callback should run exactly once",
        )?;
        check_equal(
            &self.teardown_state.load(Ordering::SeqCst),
            &SENTINEL_STATE,
            "teardown should receive the state produced by setup",
        )?;
        Ok(())
    }

    /// Assert teardown was the final event and every required processing event
    /// was recorded before it.
    ///
    /// Teardown must not merely run: it must run after the transport activity
    /// that constitutes stream processing. Recording both sides in one ordered
    /// log catches a regression that reorders teardown ahead of processing.
    fn assert_teardown_after(&self, required: &[LifecycleEvent]) -> TestResult<()> {
        let events = self.events();
        let teardown_positions: Vec<usize> = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| (*event == LifecycleEvent::Teardown).then_some(index))
            .collect();
        check_equal(
            &teardown_positions.len(),
            &1usize,
            "teardown should record exactly one event",
        )?;
        let Some(teardown_index) = teardown_positions.first().copied() else {
            return Ok(check(
                false,
                format!("teardown never recorded an event: {events:?}"),
            )?);
        };
        check_equal(
            &teardown_index,
            &events.len().saturating_sub(1),
            "teardown should be the final event recorded",
        )?;
        for required_event in required {
            check(
                events
                    .iter()
                    .take(teardown_index)
                    .any(|event| event == required_event),
                format!("expected {required_event:?} before teardown, recorded {events:?}"),
            )?;
        }
        Ok(())
    }
}

/// In-memory duplex transport that records ordered lifecycle events.
///
/// Reads replay `input` and then report end of stream. Writes either fail with
/// a scripted [`io::ErrorKind::BrokenPipe`] or are captured in `output`.
struct ScriptedStream {
    input: Vec<u8>,
    read_pos: usize,
    write_fails: bool,
    events: Arc<Mutex<Vec<LifecycleEvent>>>,
    output: Arc<Mutex<Vec<u8>>>,
}

impl ScriptedStream {
    /// Serve `input` on the read half and record transport events in `events`.
    fn new(input: Vec<u8>, write_fails: bool, events: Arc<Mutex<Vec<LifecycleEvent>>>) -> Self {
        Self {
            input,
            read_pos: 0,
            write_fails,
            events,
            output: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Share the buffer that captures bytes written by the frame pipeline.
    fn output(&self) -> Arc<Mutex<Vec<u8>>> { Arc::clone(&self.output) }
}

impl AsyncRead for ScriptedStream {
    /// Record the read, replay the next chunk of scripted input, and report
    /// end of stream once the input is exhausted.
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        lock_recover(&this.events).push(LifecycleEvent::TransportRead);
        let remaining = this.input.get(this.read_pos..).unwrap_or_default();
        let take = remaining.len().min(buf.remaining());
        if let Some(chunk) = remaining.get(..take) {
            buf.put_slice(chunk);
        }
        this.read_pos += take;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for ScriptedStream {
    /// Record the write attempt, then fail or capture the bytes according to
    /// the scenario's script.
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        lock_recover(&this.events).push(LifecycleEvent::TransportWrite);
        if this.write_fails {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "scripted write failure",
            )));
        }
        lock_recover(&this.output).extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    /// Report a successful flush without touching the captured output.
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    /// Report a successful shutdown; the scripted streams own no resources.
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl ScriptedStream {
    /// Drive the prepared application and return the captured response bytes.
    async fn drive(self, prepared: &LifecyclePreparedApp) -> io::Result<Vec<u8>> {
        let output = self.output();
        prepared.handle_connection_result(self).await?;
        let response = lock_recover(&output).clone();
        Ok(response)
    }
}

/// Teardown must run after a truncated frame ends processing with an error.
#[tokio::test]
async fn teardown_runs_once_after_truncated_frame() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;
    let stream = ScriptedStream::new(vec![0, 0, 0, 2, 1], false, Arc::clone(&probe.events));

    let error = prepared
        .handle_connection_result(stream)
        .await
        .expect_err("truncated frame should fail processing");

    check_equal(
        &error.kind(),
        &io::ErrorKind::UnexpectedEof,
        "truncated frame should end with unexpected EOF",
    )?;
    probe.assert_exactly_once()?;
    probe.assert_teardown_after(&[LifecycleEvent::TransportRead])
}

/// Teardown must run after the decode-failure limit closes the connection.
#[tokio::test]
async fn teardown_runs_once_after_decode_failure_limit() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;
    let mut input = Vec::new();
    for _ in 0..DECODE_FAILURE_LIMIT {
        input.extend_from_slice(&MALFORMED_FRAME);
    }
    let stream = ScriptedStream::new(input, false, Arc::clone(&probe.events));

    let error = prepared
        .handle_connection_result(stream)
        .await
        .expect_err("repeated malformed frames should close the connection");

    check_equal(
        &error.kind(),
        &io::ErrorKind::InvalidData,
        "decode-failure limit should report invalid data",
    )?;
    check(
        error
            .to_string()
            .contains("too many deserialization failures"),
        format!("unexpected decode-failure message: {error}"),
    )?;
    probe.assert_exactly_once()?;
    probe.assert_teardown_after(&[LifecycleEvent::TransportRead])
}

/// Teardown must run after a response write fails during stream processing.
#[tokio::test]
async fn teardown_runs_once_after_response_send_failure() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;
    let stream = ScriptedStream::new(build_frame(1, vec![b'X'])?, true, Arc::clone(&probe.events));

    let error = prepared
        .handle_connection_result(stream)
        .await
        .expect_err("a failed response write should surface as an error");

    check(
        error.to_string().contains("scripted write failure"),
        format!("unexpected send error: {error}"),
    )?;
    probe.assert_exactly_once()?;
    probe.assert_teardown_after(&[
        LifecycleEvent::TransportRead,
        LifecycleEvent::TransportWrite,
    ])
}

/// Teardown must still run exactly once after a connection completes cleanly.
#[tokio::test]
async fn teardown_runs_once_after_clean_completion() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;
    let mut input = MALFORMED_FRAME.to_vec();
    input.extend_from_slice(&build_frame(1, vec![b'X'])?);
    let stream = ScriptedStream::new(input, false, Arc::clone(&probe.events));

    let response = stream.drive(&prepared).await?;

    check_equal(
        &response_payload(&response)?,
        b"X",
        "clean connection should return the handler response",
    )?;
    probe.assert_exactly_once()?;
    probe.assert_teardown_after(&[
        LifecycleEvent::TransportRead,
        LifecycleEvent::TransportWrite,
    ])
}
