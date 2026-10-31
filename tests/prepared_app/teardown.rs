//! Teardown regression coverage for returned stream-processing errors.
//!
//! Each scenario drives a prepared application far enough for setup to produce
//! state, then terminates the connection through a decode or send failure. The
//! teardown hook must still run exactly once and receive that same state.

use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use wireframe::{
    app::{Envelope, PreparedApp, WireframeApp},
    serializer::BincodeSerializer,
};
use wireframe_testing::{TestResult, drive_prepared_with_frames};

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

/// Shared observations recorded by the lifecycle hooks under test.
#[derive(Clone)]
struct LifecycleProbe {
    setup_calls: Arc<AtomicUsize>,
    teardown_calls: Arc<AtomicUsize>,
    teardown_state: Arc<AtomicU32>,
}

impl LifecycleProbe {
    /// Create counters that start at zero.
    fn new() -> Self {
        Self {
            setup_calls: Arc::new(AtomicUsize::new(0)),
            teardown_calls: Arc::new(AtomicUsize::new(0)),
            teardown_state: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Build an application whose setup returns the sentinel and whose
    /// teardown records both its call count and the state it receives.
    fn app(&self) -> TestResult<LifecycleApp> {
        let setup_calls = Arc::clone(&self.setup_calls);
        let teardown_calls = Arc::clone(&self.teardown_calls);
        let teardown_state = Arc::clone(&self.teardown_state);

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
                async move {
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
}

/// In-memory transport that yields one scripted read and fails every write.
struct FailingWriteStream {
    bytes: Vec<u8>,
    read_pos: usize,
}

impl FailingWriteStream {
    /// Serve `bytes` on the read half, then report end of stream.
    fn new(bytes: Vec<u8>) -> Self { Self { bytes, read_pos: 0 } }
}

impl AsyncRead for FailingWriteStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let remaining = &self.bytes[self.read_pos..];
        let take = remaining.len().min(buf.remaining());
        buf.put_slice(&remaining[..take]);
        self.read_pos += take;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for FailingWriteStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "scripted write failure",
        )))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn teardown_runs_once_after_truncated_frame() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;

    let error = drive_prepared_with_frames(&prepared, vec![vec![0, 0, 0, 2, 1]])
        .await
        .expect_err("truncated frame should fail processing");

    check_equal(
        &error.kind(),
        &io::ErrorKind::UnexpectedEof,
        "truncated frame should end with unexpected EOF",
    )?;
    probe.assert_exactly_once()
}

#[tokio::test]
async fn teardown_runs_once_after_decode_failure_limit() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;
    let frames = vec![MALFORMED_FRAME.to_vec(); DECODE_FAILURE_LIMIT];

    let error = drive_prepared_with_frames(&prepared, frames)
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
    probe.assert_exactly_once()
}

#[tokio::test]
async fn teardown_runs_once_after_response_send_failure() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;
    let stream = FailingWriteStream::new(build_frame(1, vec![b'X'])?);

    let error = prepared
        .handle_connection_result(stream)
        .await
        .expect_err("a failed response write should surface as an error");

    check(
        error.to_string().contains("scripted write failure"),
        format!("unexpected send error: {error}"),
    )?;
    probe.assert_exactly_once()
}

#[tokio::test]
async fn teardown_runs_once_after_clean_completion() -> TestResult<()> {
    let probe = LifecycleProbe::new();
    let prepared = probe.prepare().await?;

    let response = drive_prepared_with_frames(
        &prepared,
        vec![MALFORMED_FRAME.to_vec(), build_frame(1, vec![b'X'])?],
    )
    .await?;

    check_equal(
        &response_payload(&response)?,
        &[b'X'],
        "clean connection should return the handler response",
    )?;
    probe.assert_exactly_once()
}
