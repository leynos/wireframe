//! Ordering coverage for dispatch, response writes, and teardown.
//!
//! Cancellation is observed only at the next read boundary, so a frame that is
//! already being dispatched must finish and have its response written before
//! teardown starts. These tests record those side effects in the order the
//! runtime produced them, rather than inferring the order from the final byte
//! count.

use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf, duplex},
    sync::Notify,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

use super::tests::{drive, request_frame};
use crate::{
    app::{Envelope, Handler, Packet, PreparedApp, WireframeApp},
    message::Message,
    serializer::BincodeSerializer,
};

/// Prepared application whose connection state is the unit type.
type TestPrepared = PreparedApp<BincodeSerializer, (), Envelope>;

/// One observable connection event, in the order the runtime produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionEvent {
    /// Application bytes were accepted by the transport.
    Write,
    /// The teardown callback started.
    Teardown,
}

/// Shared, ordered record of [`ConnectionEvent`]s.
#[derive(Clone, Default)]
struct EventLog {
    /// Events in production order.
    events: Arc<Mutex<Vec<ConnectionEvent>>>,
}

impl EventLog {
    /// Append one event.
    fn record(&self, event: ConnectionEvent) {
        self.events.lock().expect("event log mutex").push(event);
    }

    /// Copy the events recorded so far.
    fn snapshot(&self) -> Vec<ConnectionEvent> {
        self.events.lock().expect("event log mutex").clone()
    }
}

/// Transport wrapper that records every write the runtime completes.
///
/// The wrapper waits until the inner transport accepts at least one byte, so a
/// pending write is not mistaken for a completed one.
struct WriteRecorder<W> {
    /// Wrapped transport half.
    inner: W,
    /// Shared event log receiving one entry per accepted write.
    events: EventLog,
}

impl<W> WriteRecorder<W> {
    /// Wrap `inner`, recording accepted writes into `events`.
    fn new(inner: W, events: EventLog) -> Self { Self { inner, events } }
}

impl<W: AsyncRead + Unpin> AsyncRead for WriteRecorder<W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for WriteRecorder<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if matches!(&result, Poll::Ready(Ok(written)) if *written > 0) {
            self.events.record(ConnectionEvent::Write);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Gates and observations shared with the dispatch ordering test.
struct DispatchScenario {
    /// Teardown invocations observed by the test.
    calls: Arc<AtomicUsize>,
    /// Ordered write and teardown events.
    events: EventLog,
    /// Notified when the handler starts.
    started: Arc<Notify>,
    /// Released by the test to let the handler finish.
    release: Arc<Notify>,
}

impl DispatchScenario {
    /// Create a scenario with no handler call or teardown recorded yet.
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            events: EventLog::default(),
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }

    /// Build the gated application, recording teardown into the event log.
    async fn prepare(&self) -> TestPrepared {
        let handler_started = Arc::clone(&self.started);
        let handler_release = Arc::clone(&self.release);
        let handler: Handler<Envelope> = Arc::new(move |_: &Envelope| {
            let started_gate = Arc::clone(&handler_started);
            let release_gate = Arc::clone(&handler_release);
            Box::pin(async move {
                started_gate.notify_one();
                release_gate.notified().await;
            })
        });
        let teardown_events = self.events.clone();
        let teardown_calls = Arc::clone(&self.calls);
        WireframeApp::<BincodeSerializer, (), Envelope>::new()
            .expect("build application")
            .route(1, handler)
            .expect("register route")
            .on_connection_setup(|| async {})
            .expect("register setup hook")
            .on_connection_teardown(move |()| {
                let recorded = teardown_events.clone();
                let counter = Arc::clone(&teardown_calls);
                async move {
                    recorded.record(ConnectionEvent::Teardown);
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })
            .expect("register teardown hook")
            .prepare()
            .await
            .expect("prepare application")
    }

    /// Copy the events recorded so far.
    fn snapshot(&self) -> Vec<ConnectionEvent> { self.events.snapshot() }
}

/// Cancellation during dispatch lets the in-flight response finish first.
///
/// The handler is gated so the test can cancel while it is running. The
/// runtime observes cancellation only at the next read boundary, so the
/// response must still be written and teardown must wait for dispatch. The
/// recorded event log proves the write happened before teardown started, which
/// a byte-count assertion alone cannot show.
#[tokio::test]
async fn cancellation_during_dispatch_completes_the_response_before_teardown() {
    let scenario = DispatchScenario::new();
    let prepared = scenario.prepare().await;
    let token = CancellationToken::new();
    let canceller = token.clone();
    let (mut client, server) = duplex(64);
    let recorded = WriteRecorder::new(server, scenario.events.clone());
    let connection = tokio::spawn(async move { drive(&prepared, recorded, Some(token)).await });

    client
        .write_all(&request_frame(1))
        .await
        .expect("write request frame");
    timeout(Duration::from_secs(5), scenario.started.notified())
        .await
        .expect("handler should start");
    canceller.cancel();
    sleep(Duration::from_millis(20)).await;
    assert_eq!(
        scenario.calls.load(Ordering::SeqCst),
        0,
        "teardown must wait for the in-flight dispatch"
    );

    scenario.release.notify_one();
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .await
        .expect("read response");
    connection
        .await
        .expect("join connection task")
        .expect("connection should finish");

    assert_single_response(&response);
    assert_writes_precede_teardown(&scenario.snapshot());
    assert_eq!(
        scenario.calls.load(Ordering::SeqCst),
        1,
        "teardown should run once after dispatch completes"
    );
}

/// Assert the response is one complete frame answering the request route.
fn assert_single_response(response: &[u8]) {
    let frames = wireframe_testing::decode_frames(response).expect("decode response frames");
    assert_eq!(
        frames.len(),
        1,
        "the in-flight response should be exactly one complete frame"
    );
    let frame = frames
        .first()
        .expect("the response should be exactly one frame");
    let (envelope, consumed) = Envelope::from_bytes(frame).expect("decode response envelope");
    assert_eq!(
        consumed,
        frame.len(),
        "the response frame should be fully consumed"
    );
    assert_eq!(
        envelope.id(),
        1,
        "the response should answer the request route"
    );
}

/// Assert the response write was recorded before teardown started.
fn assert_writes_precede_teardown(events: &[ConnectionEvent]) {
    let mut saw_write = false;
    let mut saw_teardown = false;
    for event in events {
        match event {
            ConnectionEvent::Write if saw_teardown => {
                panic!("the response write must precede teardown: {events:?}");
            }
            ConnectionEvent::Write => saw_write = true,
            ConnectionEvent::Teardown => {
                assert!(
                    !saw_teardown,
                    "teardown should be recorded once: {events:?}"
                );
                saw_teardown = true;
            }
        }
    }
    assert!(
        saw_write,
        "the response should be written before teardown: {events:?}"
    );
    assert!(saw_teardown, "teardown should be recorded: {events:?}");
}
