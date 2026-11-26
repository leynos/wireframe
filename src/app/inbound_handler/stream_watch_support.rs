//! Shared plumbing for the peer-observation test modules.
//!
//! Both watch modules drive the production connection loop over one in-memory
//! transport, so both need the same prepared application, client, and
//! bounded-await helpers. They live here so each test module reads as the
//! scenario it proves rather than as its setup. The clock is paused in every
//! scenario, because the bound that releases a stalled reply is the actor's
//! thirty-second peer-close grace and no test may wait for it in real time.

use std::{future::Future, io, time::Duration};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::{io::DuplexStream, sync::Notify, task::JoinHandle, time::timeout};
use tokio_util::{
    codec::{Framed, LengthDelimitedCodec},
    sync::CancellationToken,
};
use wireframe_testing::{TestError, TestResult};

use super::super::{ConnectionProcessingContext, process_connection};
use crate::{
    app::{Envelope, PreparedApp, WireframeApp},
    serializer::{BincodeSerializer, Serializer},
    test_helpers::first_frame_payload,
};

/// Application type every watch scenario builds.
pub(super) type App = WireframeApp<BincodeSerializer, (), Envelope>;

/// Prepared form of [`App`], as the connection runtime receives it.
type Prepared = PreparedApp<BincodeSerializer, (), Envelope>;

/// Client end of one in-memory connection.
pub(super) type Client = Framed<DuplexStream, LengthDelimitedCodec>;

/// Bytes the in-memory transport may buffer in each direction.
const DUPLEX_CAPACITY: usize = 1024;

/// Bound for a step that has to complete on its own.
///
/// The clock is paused, so this only fires when the connection genuinely makes
/// no progress, and the failure names the step that stalled.
pub(super) const STEP_WAIT: Duration = Duration::from_secs(5);

/// Slack allowed past the grace for scheduling at one virtual instant.
pub(super) const GRACE_SLACK: Duration = Duration::from_millis(1);

/// Correlation identifier stamped on every request in these modules.
const REQUEST_CORRELATION: u64 = 5;

/// Require `condition`, reporting `message` as the test failure otherwise.
pub(super) fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// One spawned connection and its client end.
pub(super) struct Scenario {
    /// Client end of the in-memory transport.
    pub(super) client: Client,
    /// Task driving the production connection loop.
    pub(super) connection: JoinHandle<io::Result<()>>,
}

/// Prepare `app` and spawn its connection over an in-memory transport.
pub(super) async fn spawn_connection(app: App) -> TestResult<Scenario> {
    spawn_connection_buffering(app, DUPLEX_CAPACITY).await
}

/// Prepare `app` and spawn its connection over a transport that buffers
/// `capacity` bytes in each direction.
///
/// A scenario that needs a reply to block mid-write uses a capacity smaller
/// than that reply; a reply larger than the transport buffer cannot be written
/// in one go.
pub(super) async fn spawn_connection_buffering(app: App, capacity: usize) -> TestResult<Scenario> {
    let prepared: Prepared = app
        .prepare()
        .await
        .map_err(|error| format!("prepare the application: {error}"))?;
    let (client, server) = tokio::io::duplex(capacity);
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
        serializer: prepared.serializer.as_ref(),
        codec: &prepared.codec,
        on_connect: prepared.on_connect.as_deref(),
        on_disconnect: prepared.on_disconnect.as_deref(),
        message_assembler: prepared.message_assembler.as_deref(),
        fragmentation: prepared.fragmentation,
        protocol_hooks: prepared.protocol_hooks(),
        memory_budgets: prepared.memory_budgets,
        read_timeout_ms: prepared.read_timeout_ms,
        shutdown,
        push_queues: prepared.push_queues(),
        session_registry: std::sync::Arc::clone(&prepared.session_registry),
        // The scenario talks to the server over an in-memory duplex, so there
        // is no accepted socket and therefore no peer address to report.
        peer_addr: None,
        recovery_hook: prepared.recovery_hook.as_deref(),
        recovery_config: prepared.recovery_config.clone(),
    }
}

/// Send one request envelope carrying `payload` to `route`.
pub(super) async fn send_request(client: &mut Client, route: u32, payload: &[u8]) -> TestResult {
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
pub(super) async fn send_complete(
    client: &mut Client,
    route: u32,
    key: u64,
    body: &[u8],
) -> TestResult {
    send_request(
        client,
        route,
        &first_frame_payload(key.into(), body, true, None)?,
    )
    .await
}

/// Read one reply frame and return its payload.
pub(super) async fn read_reply(client: &mut Client, step: &str) -> TestResult<Vec<u8>> {
    read_reply_within(STEP_WAIT, client, step).await
}

/// Read one reply frame that is only due once a longer bound has passed.
///
/// A reply held back by the stall bound cannot arrive within [`STEP_WAIT`], so
/// the caller supplies its own bound instead of letting the read time out
/// first.
pub(super) async fn read_reply_within(
    wait: Duration,
    client: &mut Client,
    step: &str,
) -> TestResult<Vec<u8>> {
    let frame = bounded_for(wait, step, client.next())
        .await?
        .ok_or_else(|| format!("the connection closed before {step} arrived"))??;
    let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(&frame)?;
    Ok(envelope.payload_bytes().to_vec())
}

/// Open a streaming body that owns the dispatch slot until its handler is
/// released.
pub(super) async fn open_body(
    client: &mut Client,
    route: u32,
    key: u64,
    started: &Notify,
) -> TestResult {
    send_request(
        client,
        route,
        &first_frame_payload(key.into(), b"start", false, None)?,
    )
    .await?;
    bounded("the live handler starting", started.notified()).await
}

/// Await `future`, bounding it by [`STEP_WAIT`].
pub(super) async fn bounded<T>(step: &str, future: impl Future<Output = T>) -> TestResult<T> {
    bounded_for(STEP_WAIT, step, future).await
}

/// Await `future`, bounding it by `wait`.
pub(super) async fn bounded_for<T>(
    wait: Duration,
    step: &str,
    future: impl Future<Output = T>,
) -> TestResult<T> {
    timeout(wait, future)
        .await
        .map_err(|_| format!("{step} did not complete within {wait:?}").into())
}
