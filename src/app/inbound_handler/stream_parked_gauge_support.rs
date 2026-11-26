//! Setup shared by the parked-gauge scenarios.
//!
//! A scenario needs the same application, handlers and paired connections;
//! the pieces live here so each scenario reads as the ending it drives and
//! the scheduler stays small enough to review.

use super::*;

/// What a streaming handler does once its body is released.
#[derive(Clone, Copy)]
pub(super) enum AfterRelease {
    /// Drain the body and answer, which is the ordinary ending.
    Reply,
    /// Unwind through the connection task, which is the ending under test.
    Panic,
}

/// Read a body to its end and answer it, failing if the peer truncated it.
pub(super) async fn answer_body(body: &mut RequestBodyStream) -> Option<Bytes> {
    while let Some(item) = body.next().await {
        item.ok()?;
    }
    Some(Bytes::from_static(b"live"))
}

/// Handler that holds the dispatch slot until released, then does `finish`.
///
/// Both endings share their whole setup: the handler signals that it started,
/// waits for the release, and only then behaves differently. Keeping the shared
/// part in one function is what lets each ending read as the one line that
/// distinguishes it.
pub(super) fn holding_handler(
    started: Arc<Notify>,
    release: Arc<Notify>,
    finish: AfterRelease,
) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, mut body: RequestBodyStream| {
        let start_signal = Arc::clone(&started);
        let release_signal = Arc::clone(&release);
        Box::pin(async move {
            start_signal.notify_one();
            release_signal.notified().await;
            match finish {
                AfterRelease::Reply => answer_body(&mut body).await,
                AfterRelease::Panic => panic!("the streaming handler panicked"),
            }
        })
    })
}

/// Reply handler for the parked route that holds one reply until released.
///
/// The drain answers parked messages one at a time, so holding the first reply
/// back freezes the gauge at the count the drain has already removed.
pub(super) fn parked_reply(gated: Arc<Notify>, release: Arc<Notify>) -> ResponseHandler<Envelope> {
    Arc::new(move |request: &Envelope| {
        let is_gated = request.payload_bytes() == GATED_PAYLOAD;
        let gated_signal = Arc::clone(&gated);
        let release_signal = Arc::clone(&release);
        Box::pin(async move {
            if is_gated {
                gated_signal.notify_one();
                release_signal.notified().await;
            }
            Response::Single(Envelope::new(PARKED_ROUTE, None, b"done".to_vec()))
        })
    })
}

/// Build the two-route application an ending drives.
pub(super) fn scenario_app(
    handler: StreamingHandler,
    gated: Arc<Notify>,
    release: Arc<Notify>,
) -> TestResult<App> {
    WireframeApp::new()
        .map(|app| app.with_message_assembler(TestAssembler))
        .and_then(|app| app.streaming_route(LIVE_ROUTE, handler))
        .and_then(|app| app.route_response(PARKED_ROUTE, parked_reply(gated, release)))
        .map_err(|error| TestError::Msg(format!("build the application: {error}")))
}

/// Park one complete message behind the live body.
pub(super) async fn park_one(client: &mut Client, key: u64, body: &[u8]) -> TestResult {
    send_complete(client, PARKED_ROUTE, key, body).await
}

/// One connection under test, with two messages parked behind a live body.
pub(super) struct Paired {
    /// Client end of the connection under test.
    pub(super) client: Client,
    /// Task driving it, so an ending can abort or await it.
    pub(super) connection: JoinHandle<io::Result<()>>,
    /// Frees the live handler when notified.
    pub(super) release: Arc<Notify>,
    /// Signalled when the drain reaches the gated parked reply.
    pub(super) gated: Arc<Notify>,
    /// Frees the gated parked reply when notified.
    gate_release: Arc<Notify>,
}

/// Spawn the connection under test and park two messages behind its body.
///
/// `finish` selects what the live handler does when it is released, so the
/// panic ending shares this setup rather than repeating it.
pub(super) async fn parked_pair(finish: AfterRelease) -> TestResult<Paired> {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let gated = Arc::new(Notify::new());
    let gate_release = Arc::new(Notify::new());
    let app = scenario_app(
        holding_handler(Arc::clone(&started), Arc::clone(&release), finish),
        Arc::clone(&gated),
        Arc::clone(&gate_release),
    )?;
    let Scenario {
        mut client,
        connection,
    } = spawn_connection(app).await?;
    open_body(&mut client, LIVE_ROUTE, LIVE_KEY, &started).await?;
    park_one(&mut client, FIRST_KEY, GATED_PAYLOAD).await?;
    park_one(&mut client, SECOND_KEY, SECOND_PAYLOAD).await?;
    Ok(Paired {
        client,
        connection,
        release,
        gated,
        gate_release,
    })
}

/// Free the live body and read its reply, stopping when the drain starts.
///
/// The drain pops the gated message first and holds its reply, which is what
/// makes the intermediate parked count observable.
pub(super) async fn answer_live_body(paired: &mut Paired) -> TestResult {
    paired.release.notify_one();
    send_request(
        &mut paired.client,
        LIVE_ROUTE,
        &continuation_frame_payload(LIVE_KEY.into(), 1.into(), b"end", true)?,
    )
    .await?;
    ensure(
        read_reply(&mut paired.client, "the live reply").await? == b"live".to_vec(),
        "the live body must be answered before the parked messages",
    )?;
    bounded_for(
        STEP_WAIT,
        "the drain reaching its first reply",
        paired.gated.notified(),
    )
    .await
}

/// Release the gated reply and read both parked replies.
pub(super) async fn answer_parked_pair(paired: &mut Paired) -> TestResult {
    paired.gate_release.notify_one();
    ensure(
        read_reply(&mut paired.client, "the first parked reply").await? == b"done".to_vec(),
        "the gated parked message must be answered once released",
    )?;
    ensure(
        read_reply(&mut paired.client, "the second parked reply").await? == b"done".to_vec(),
        "the second parked message must be answered",
    )
}

/// A second connection kept alive with one message parked.
///
/// Its parked count is the baseline every scenario measures against. Without
/// it, an extra release on the connection under test would take the gauge to
/// zero, which is where a gauge saturates: the mistake would be invisible. The
/// connection is held for the rest of the scenario because dropping it would
/// end the connection and release the sentinel's count.
pub(super) struct Sentinel {
    /// The parked connection; dropped only after the assertions.
    _connection: Scenario,
    /// Live handler's start signal, held so the body stays open.
    _started: Arc<Notify>,
    /// Release signal, deliberately never notified.
    _release: Arc<Notify>,
}

/// Spawn the sentinel connection and park one message on it.
pub(super) async fn parked_sentinel() -> TestResult<Sentinel> {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = scenario_app(
        holding_handler(
            Arc::clone(&started),
            Arc::clone(&release),
            AfterRelease::Reply,
        ),
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    )?;
    let mut connection = spawn_connection(app).await?;
    open_body(
        &mut connection.client,
        LIVE_ROUTE,
        SENTINEL_LIVE_KEY,
        &started,
    )
    .await?;
    park_one(&mut connection.client, SENTINEL_PARKED_KEY, b"sentinel").await?;
    Ok(Sentinel {
        _connection: connection,
        _started: started,
        _release: release,
    })
}
