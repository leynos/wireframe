//! Check that the cancellation trace events record `reason` as a string.
//!
//! The workspace test suite cannot make this check: its `wireframe_testing`
//! development dependency builds `wireframe` with default features, which
//! enables `metrics`. This consumer is compiled with `metrics` off, so it runs
//! a real server and inspects the structured fields a subscriber receives.
//! Changing `reason` from a string value to a `Display` value, which `tracing`
//! records through `record_debug`, breaks the documented subscriber contract
//! even though the formatted text looks the same.

use std::{
    error::Error,
    fmt::Debug,
    future::{self, Future},
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::{
    net::TcpStream,
    runtime::Builder,
    sync::oneshot,
    time::{sleep, timeout},
};
use tracing::{
    Event,
    Subscriber,
    field::{Field, Visit},
    subscriber::with_default,
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt},
    registry,
};
use wireframe::{app::WireframeApp, server::WireframeServer};

/// Result type for the helpers below.
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

/// How a subscriber received a field value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordedAs {
    /// Through `Visit::record_str`.
    Str,
    /// Through `Visit::record_debug`.
    Debug,
}

/// A captured field value and the visitor method that received it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CapturedValue {
    /// Visitor method that recorded the value.
    recorded_as: RecordedAs,
    /// Rendered value.
    value: String,
}

/// The fields of interest from one trace event.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CapturedEvent {
    /// The `event` field, when the event carries one.
    event: Option<String>,
    /// The `reason` field and how it was recorded, when present.
    reason: Option<CapturedValue>,
}

/// Collects the `event` and `reason` fields of a single event.
struct FieldVisitor<'a> {
    /// Destination for the collected fields.
    capture: &'a mut CapturedEvent,
}

impl FieldVisitor<'_> {
    /// Record a field this test cares about, ignoring all others.
    fn record(&mut self, field: &Field, value: &str, recorded_as: RecordedAs) {
        match field.name() {
            "event" => self.capture.event = Some(value.to_owned()),
            "reason" => {
                self.capture.reason = Some(CapturedValue {
                    recorded_as,
                    value: value.to_owned(),
                });
            }
            _ => {}
        }
    }
}

impl Visit for FieldVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value, RecordedAs::Str);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record(field, &format!("{value:?}"), RecordedAs::Debug);
    }
}

/// Subscriber layer that retains every event it observes.
#[derive(Clone, Default)]
struct EventCapture {
    /// Events observed so far.
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl EventCapture {
    /// Return the events observed so far, ignoring a poisoned lock.
    #[must_use]
    fn snapshot(&self) -> Vec<CapturedEvent> {
        let Ok(events) = self.events.lock() else {
            return Vec::new();
        };
        events.to_vec()
    }
}

impl<S: Subscriber> Layer<S> for EventCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut captured = CapturedEvent::default();
        event.record(&mut FieldVisitor {
            capture: &mut captured,
        });
        let Ok(mut events) = self.events.lock() else {
            return;
        };
        events.push(captured);
    }
}

/// Bind a listener to an ephemeral loopback port.
fn unused_listener() -> TestResult<StdTcpListener> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener)
}

/// Wait for the server supervisor to report readiness.
async fn wait_for_readiness(readiness: oneshot::Receiver<()>) -> TestResult {
    timeout(Duration::from_secs(1), readiness)
        .await
        .map_err(|_| "server did not reach readiness")?
        .map_err(|error| format!("server dropped readiness sender: {error}"))?;
    Ok(())
}

/// Wait until the server listener refuses new connections.
async fn wait_for_listener_release(address: SocketAddr) -> TestResult {
    let release_result: TestResult = timeout(Duration::from_secs(1), async {
        loop {
            sleep(Duration::from_millis(10)).await;
            match TcpStream::connect(address).await {
                Ok(stream) => drop(stream),
                Err(error) if error.kind() == ErrorKind::ConnectionRefused => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
    })
    .await
    .map_err(|_| format!("server listener at {address} remained bound for one second"))?;
    release_result
}

/// Run a server until `shutdown` completes.
async fn run_server_until<F>(
    listener: StdTcpListener,
    ready: oneshot::Sender<()>,
    shutdown: F,
) -> TestResult
where
    F: Future<Output = ()> + Send,
{
    let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
        .workers(1)
        .bind_existing_listener(listener)?;
    server
        .ready_signal(ready)
        .run_with_shutdown(shutdown)
        .await?;
    Ok(())
}

/// Run a server that stops only when its future is dropped.
async fn run_server_until_dropped(
    listener: StdTcpListener,
    ready: oneshot::Sender<()>,
) -> TestResult {
    run_server_until(listener, ready, future::pending()).await
}

/// Wait for readiness, failing if the supervisor finishes first.
async fn wait_for_supervisor_readiness<F>(
    readiness: &mut oneshot::Receiver<()>,
    supervisor: &mut Pin<Box<F>>,
) -> TestResult
where
    F: Future<Output = TestResult>,
{
    timeout(Duration::from_secs(1), async {
        tokio::select! {
            ready = readiness => ready.map_err(|error| {
                format!("server dropped readiness sender: {error}").into()
            }),
            result = supervisor => Err(format!(
                "server supervisor completed before readiness: {result:?}"
            ).into()),
        }
    })
    .await
    .map_err(|_| "server did not reach readiness")?
}

/// Run a server through its graceful-shutdown path and return the events.
fn capture_graceful_events() -> TestResult<Vec<CapturedEvent>> {
    let capture = EventCapture::default();
    let subscriber = registry().with(capture.clone());
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let outcome: TestResult = with_default(subscriber, || {
        runtime.block_on(async {
            let listener = unused_listener()?;
            let address = listener.local_addr()?;
            let (ready_tx, ready_rx) = oneshot::channel();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let handle = tokio::spawn(run_server_until(listener, ready_tx, async {
                drop(shutdown_rx.await);
            }));

            wait_for_readiness(ready_rx).await?;
            shutdown_tx
                .send(())
                .map_err(|()| "server shutdown receiver was dropped")?;
            let join_result = timeout(Duration::from_secs(1), handle)
                .await
                .map_err(|_| "graceful server did not complete in time")?;
            join_result.map_err(|error| format!("server task failed: {error}"))??;
            wait_for_listener_release(address).await
        })
    });
    outcome.map(|()| capture.snapshot())
}

/// Run a server, drop its supervisor, and return the events.
fn capture_dropped_events() -> TestResult<Vec<CapturedEvent>> {
    let capture = EventCapture::default();
    let subscriber = registry().with(capture.clone());
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let outcome: TestResult = with_default(subscriber, || {
        runtime.block_on(async {
            let listener = unused_listener()?;
            let address = listener.local_addr()?;
            let (ready_tx, mut ready_rx) = oneshot::channel();
            let mut supervisor = Box::pin(run_server_until_dropped(listener, ready_tx));
            wait_for_supervisor_readiness(&mut ready_rx, &mut supervisor).await?;
            drop(supervisor);
            wait_for_listener_release(address).await
        })
    });
    outcome.map(|()| capture.snapshot())
}

/// Assert every matching event recorded `reason` through `record_str`.
fn assert_reason_recorded(events: &[CapturedEvent], event_name: &str, reason: &str) {
    let matched: Vec<&CapturedValue> = events
        .iter()
        .filter(|event| event.event.as_deref() == Some(event_name))
        .filter_map(|event| event.reason.as_ref())
        .collect();
    assert!(
        !matched.is_empty(),
        "no {event_name} event carried a reason field; captured: {events:?}"
    );
    for value in matched {
        assert_eq!(
            value.recorded_as,
            RecordedAs::Str,
            "{event_name} must record reason through record_str, not record_debug"
        );
        assert_eq!(
            value.value, reason,
            "{event_name} event reported an unexpected reason"
        );
    }
}

/// The graceful path must record `reason` as a string for both events.
#[test]
fn graceful_cancellation_records_reason_as_a_string() {
    let events = capture_graceful_events().unwrap_or_else(|error| {
        panic!("graceful server run failed: {error}");
    });
    assert_reason_recorded(&events, "server_supervisor_cancellation", "graceful");
    assert_reason_recorded(&events, "server_accept_loop_exited", "graceful");
}

/// The dropped path must record `reason` as a string for both events.
#[test]
fn dropped_cancellation_records_reason_as_a_string() {
    let events = capture_dropped_events().unwrap_or_else(|error| {
        panic!("dropped server run failed: {error}");
    });
    assert_reason_recorded(&events, "server_supervisor_cancellation", "dropped");
    assert_reason_recorded(&events, "server_accept_loop_exited", "dropped");
}
