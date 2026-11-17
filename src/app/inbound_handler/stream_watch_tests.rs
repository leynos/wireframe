//! Peer observation while a parked message's reply streams.
//!
//! A message that completes while a streaming request body owns the dispatch
//! slot is parked, then answered once the slot frees. Its reply travels the
//! ordinary response path, so it may stream for an unbounded time. These tests
//! drive the production connection loop over one in-memory transport and prove
//! the peer's read half stays observed throughout: without that, a half-close
//! is never seen, the actor's bounded peer-close drain is never armed, and a
//! producer that stops yielding keeps the connection alive forever.
//!
//! The clock is paused because the bound that releases a stalled reply is the
//! actor's thirty-second peer-close grace, and no test may wait for it in real
//! time. A stalled producer holds no timer of its own, so the paused clock
//! reaches exactly the deadline under test.

use std::{future::Future, io, sync::Arc, time::Duration};

use bytes::Bytes;
use futures::{SinkExt, StreamExt, stream};
use tokio::{
    io::DuplexStream,
    sync::Notify,
    task::JoinHandle,
    time::{Instant, timeout},
};
use tokio_util::{
    codec::{Framed, LengthDelimitedCodec},
    sync::CancellationToken,
};
use wireframe_testing::{TestError, TestResult};

use super::super::{ConnectionProcessingContext, process_connection};
use crate::{
    app::{Envelope, PreparedApp, ResponseHandler, StreamingHandler, WireframeApp},
    connection::PEER_CLOSED_RESPONSE_GRACE,
    request::{RequestBodyStream, RequestParts},
    response::{FrameStream, Response, WireframeError},
    serializer::{BincodeSerializer, Serializer},
    test_helpers::{TestAssembler, continuation_frame_payload, first_frame_payload},
};

/// Route whose streaming handler holds the dispatch slot open.
const LIVE_ROUTE: u32 = 70;
/// Buffered route whose reply streams one frame and then stalls.
const PARKED_ROUTE: u32 = 71;
/// Message key of the live streaming body.
const LIVE_KEY: u64 = 41;
/// Message key of the buffered message parked behind it.
const PARKED_KEY: u64 = 42;
/// Correlation identifier stamped on every request in this module.
const REQUEST_CORRELATION: u64 = 5;
/// Bytes the in-memory transport may buffer in each direction.
const DUPLEX_CAPACITY: usize = 1024;
/// Bound for a step that has to complete on its own.
///
/// The clock is paused, so this only fires when the connection genuinely makes
/// no progress, and the failure names the step that stalled.
const STEP_WAIT: Duration = Duration::from_secs(5);
/// Slack allowed past the grace for scheduling at one virtual instant.
const GRACE_SLACK: Duration = Duration::from_millis(1);

/// Application type every scenario builds.
type App = WireframeApp<BincodeSerializer, (), Envelope>;
/// Prepared form of [`App`], as the connection runtime receives it.
type Prepared = PreparedApp<BincodeSerializer, (), Envelope>;
/// Client end of one in-memory connection.
type Client = Framed<DuplexStream, LengthDelimitedCodec>;

/// Require `condition`, reporting `message` as the test failure otherwise.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Handler that holds the slot until released, then drains its body.
///
/// A body that completes produces the live reply; one truncated at end-of-file
/// produces none, so a single handler serves both peer-close scenarios.
fn holding_handler(started: Arc<Notify>, release: Arc<Notify>) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, mut body: RequestBodyStream| {
        let start_signal = Arc::clone(&started);
        let release_signal = Arc::clone(&release);
        Box::pin(async move {
            start_signal.notify_one();
            release_signal.notified().await;
            while let Some(item) = body.next().await {
                // A body cut short by the peer still reaches end-of-stream, so
                // the read error is the only signal that no reply is possible.
                item.ok()?;
            }
            Some(Bytes::from_static(b"live"))
        })
    })
}

/// Handler whose reply yields one frame and then never yields again.
///
/// The stalled tail is what makes the connection depend on peer observation:
/// only the idle grace can end a wait for a frame that never arrives.
fn stalled_reply() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let first = Envelope::new(PARKED_ROUTE, None, vec![1]);
            let frames: FrameStream<Envelope, ()> = Box::pin(
                stream::once(async move { Ok::<_, WireframeError<()>>(first) })
                    .chain(stream::pending()),
            );
            Response::Stream(frames)
        })
    })
}

/// Build the two-route application every scenario drives.
fn scenario_app(started: Arc<Notify>, release: Arc<Notify>) -> TestResult<App> {
    WireframeApp::new()
        .map(|app| app.with_message_assembler(TestAssembler))
        .and_then(|app| app.streaming_route(LIVE_ROUTE, holding_handler(started, release)))
        .and_then(|app| app.route_response(PARKED_ROUTE, stalled_reply()))
        .map_err(|error| TestError::Msg(format!("build the application: {error}")))
}

/// One spawned connection and its client end.
struct Scenario {
    /// Client end of the in-memory transport.
    client: Client,
    /// Task driving the production connection loop.
    connection: JoinHandle<io::Result<()>>,
}

/// Prepare `app` and spawn its connection over an in-memory transport.
async fn spawn_connection(app: App) -> TestResult<Scenario> {
    let prepared: Prepared = app
        .prepare()
        .await
        .map_err(|error| format!("prepare the application: {error}"))?;
    let (client, server) = tokio::io::duplex(DUPLEX_CAPACITY);
    let connection = tokio::spawn(async move {
        let shutdown = CancellationToken::new();
        process_connection(server, context(&prepared, &shutdown)).await
    });
    Ok(Scenario {
        client: Framed::new(client, LengthDelimitedCodec::new()),
        connection,
    })
}

/// Borrow the prepared application as the runtime's processing context.
fn context<'a, C, F>(
    prepared: &'a PreparedApp<BincodeSerializer, C, Envelope, F>,
    shutdown: &'a CancellationToken,
) -> ConnectionProcessingContext<'a, BincodeSerializer, C, Envelope, F>
where
    C: Send + 'static,
    F: crate::codec::FrameCodec,
{
    ConnectionProcessingContext {
        routes: &prepared.routes,
        streaming_routes: &prepared.streaming_routes,
        serializer: Arc::clone(&prepared.serializer),
        codec: &prepared.codec,
        on_connect: prepared.on_connect.as_ref(),
        on_disconnect: prepared.on_disconnect.as_ref(),
        message_assembler: prepared.message_assembler.as_ref(),
        fragmentation: prepared.fragmentation,
        protocol_hooks: prepared.protocol_hooks(),
        protocol: prepared.protocol.clone(),
        memory_budgets: prepared.memory_budgets,
        read_timeout_ms: prepared.read_timeout_ms,
        shutdown,
    }
}

/// Send one request envelope carrying `payload` to `route`.
async fn send_request(client: &mut Client, route: u32, payload: &[u8]) -> TestResult {
    let envelope = Envelope::new(route, Some(REQUEST_CORRELATION), payload.to_vec());
    let encoded = BincodeSerializer.serialize(&envelope)?;
    client.send(Bytes::from(encoded)).await?;
    Ok(())
}

/// Send one complete message to a buffered route.
///
/// The assembler is installed for the whole connection, so a request to a
/// buffered route must still arrive as assembly frames: the payload the
/// handler receives is the assembled body, and a payload that is not a frame
/// header is a deserialization failure rather than a request.
async fn send_complete(client: &mut Client, route: u32, key: u64, body: &[u8]) -> TestResult {
    send_request(
        client,
        route,
        &first_frame_payload(key.into(), body, true, None)?,
    )
    .await
}

/// Read one reply frame and return its payload.
async fn read_reply(client: &mut Client, step: &str) -> TestResult<Vec<u8>> {
    let frame = bounded(step, client.next())
        .await?
        .ok_or_else(|| format!("the connection closed before {step} arrived"))??;
    let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(&frame)?;
    Ok(envelope.payload_bytes().to_vec())
}

/// Await `future`, bounding it by [`STEP_WAIT`].
async fn bounded<T>(step: &str, future: impl Future<Output = T>) -> TestResult<T> {
    bounded_for(STEP_WAIT, step, future).await
}

/// Await `future`, bounding it by `wait`.
async fn bounded_for<T>(
    wait: Duration,
    step: &str,
    future: impl Future<Output = T>,
) -> TestResult<T> {
    timeout(wait, future)
        .await
        .map_err(|_| format!("{step} did not complete within {wait:?}").into())
}

/// Open a streaming body that owns the dispatch slot until released.
async fn open_live_body(client: &mut Client, started: &Notify) -> TestResult {
    send_request(
        client,
        LIVE_ROUTE,
        &first_frame_payload(LIVE_KEY.into(), b"start", false, None)?,
    )
    .await?;
    bounded("the live handler starting", started.notified()).await
}

/// A parked reply observes a peer that half-closes while it streams.
///
/// The reply stalls after one frame, so nothing but observing the closed read
/// half can release the connection. It must therefore end at the peer-close
/// grace and not before it.
#[tokio::test(start_paused = true)]
async fn a_parked_reply_observes_a_peer_that_half_closes_while_it_streams() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(scenario_app(Arc::clone(&started), Arc::clone(&release))?).await?;

    open_live_body(&mut client, &started).await?;
    // A complete buffered message arrives while the live body owns the slot.
    send_complete(&mut client, PARKED_ROUTE, PARKED_KEY, b"parked").await?;

    // Free the slot: the live reply comes first, then the parked stream begins.
    release.notify_one();
    send_request(
        &mut client,
        LIVE_ROUTE,
        &continuation_frame_payload(LIVE_KEY.into(), 1.into(), b"end", true)?,
    )
    .await?;
    ensure(
        read_reply(&mut client, "the live reply").await? == b"live".to_vec(),
        "the live body must be answered before the parked one",
    )?;
    ensure(
        read_reply(&mut client, "the parked reply's first frame").await? == vec![1],
        "the parked reply must start streaming once the slot frees",
    )?;

    // The producer has stopped yielding, so only the half-close can release it.
    SinkExt::<Bytes>::close(&mut client).await?;
    let window = Instant::now();
    let joined = bounded_for(
        PEER_CLOSED_RESPONSE_GRACE + GRACE_SLACK,
        "the connection ending after the peer half-closed",
        connection,
    )
    .await?;
    joined??;
    let elapsed = window.elapsed();

    ensure(
        elapsed >= PEER_CLOSED_RESPONSE_GRACE,
        format!(
            "a stalled parked reply must survive the idle grace, not be cut short: ended after \
             {elapsed:?}"
        ),
    )?;
    ensure(
        elapsed <= PEER_CLOSED_RESPONSE_GRACE + GRACE_SLACK,
        format!("the reply must be cancelled at the grace, not later: ended after {elapsed:?}"),
    )?;
    ensure(
        client.next().await.is_none(),
        "the connection must close rather than serve a frame after the grace",
    )
}

/// A parked reply is bounded when the peer closes before the slot frees.
///
/// The half-close truncates the live body, so the drain starts after the peer
/// is already gone. Each parked reply must still be bounded rather than waiting
/// for a producer nobody can interrupt.
#[tokio::test(start_paused = true)]
async fn a_parked_reply_is_bounded_when_the_peer_closes_before_it_drains() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(scenario_app(Arc::clone(&started), Arc::clone(&release))?).await?;

    open_live_body(&mut client, &started).await?;
    // The live handler is consuming, so it observes the truncation the close
    // below causes and replies with nothing.
    release.notify_one();
    send_complete(&mut client, PARKED_ROUTE, PARKED_KEY, b"parked").await?;
    SinkExt::<Bytes>::close(&mut client).await?;

    ensure(
        read_reply(&mut client, "the parked reply's first frame").await? == vec![1],
        "the parked reply must start streaming after the truncated body fails",
    )?;
    let joined = bounded_for(
        PEER_CLOSED_RESPONSE_GRACE + GRACE_SLACK,
        "the connection ending after the peer closed",
        connection,
    )
    .await?;
    joined??;

    ensure(
        client.next().await.is_none(),
        "the connection must close rather than serve a frame after the grace",
    )
}
