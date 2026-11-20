//! Recovery-policy cases driven through a live server connection.
//!
//! Cases share their connection plumbing so each test states only the
//! behaviour that distinguishes its policy, while every case reports through
//! the same metric assertions.

use super::*;

/// Decode a reply frame and return its envelope payload.
fn reply_payload(bytes: &[u8]) -> TestResult<Vec<u8>> {
    let (envelope, _) = BincodeSerializer.deserialize::<Envelope>(bytes)?;
    Ok(envelope.payload_bytes().to_vec())
}

/// Assert the echoed reply carries the payload the client sent.
fn expect_payload(payload: &[u8]) -> TestResult {
    if payload == VALID_PAYLOAD {
        Ok(())
    } else {
        Err(format!("reply payload was {payload:?}, expected {VALID_PAYLOAD:?}").into())
    }
}

/// Assert the hook observed the accepted connection's peer address.
fn expect_peer(event: &RecoveryEvent, peer: SocketAddr) -> TestResult {
    if event.peer == Some(peer) {
        Ok(())
    } else {
        Err(format!(
            "the hook must observe the accepted connection's peer address {peer}, saw {:?}",
            event.peer
        )
        .into())
    }
}

/// Assert every callback observed the accepted connection's peer address.
fn expect_peer_on_all(events: &[RecoveryEvent], peer: SocketAddr) -> TestResult {
    events.iter().try_for_each(|event| expect_peer(event, peer))
}

/// Take the single callback the hook recorded, or report how many arrived.
fn exactly_one(recorded: &[RecoveryEvent]) -> TestResult<&RecoveryEvent> {
    match recorded {
        [event] => Ok(event),
        _ => Err(format!("expected one on_error call, got {}", recorded.len()).into()),
    }
}

/// Build a hook that records its callbacks on the returned channel.
fn recording_hook(
    policy: RecoveryPolicy,
) -> (RecordingHook, mpsc::UnboundedReceiver<RecoveryEvent>) {
    let (events, receiver) = mpsc::unbounded_channel();
    let hook = RecordingHook {
        selected: policy,
        quarantine: QUARANTINE,
        events,
    };
    (hook, receiver)
}

/// Drain the hook callbacks and assert they reported exactly `expected` policies.
fn expect_policies(
    events: &mut mpsc::UnboundedReceiver<RecoveryEvent>,
    expected: &[RecoveryPolicy],
) -> TestResult<Vec<RecoveryEvent>> {
    let recorded = drain_events(events);
    let policies: Vec<RecoveryPolicy> = recorded.iter().map(|event| event.policy).collect();
    if policies == expected {
        Ok(recorded)
    } else {
        Err(format!("expected {expected:?}, saw {policies:?}").into())
    }
}

/// Start a one-worker server for `app` and connect a client to it.
async fn connect_app<Codec>(
    app: WireframeApp<BincodeSerializer, (), Envelope, Codec>,
) -> TestResult<(RunningServer, TcpStream)>
where
    Codec: FrameCodec,
{
    let server = start_server(app).await?;
    let client = TcpStream::connect(server.address).await?;
    Ok((server, client))
}

/// Connect to a server whose hook selects `policy`.
async fn connect_with_hook(
    policy: RecoveryPolicy,
    max_consecutive_drops: u32,
) -> TestResult<(
    RunningServer,
    TcpStream,
    mpsc::UnboundedReceiver<RecoveryEvent>,
)> {
    let (hook, events) = recording_hook(policy);
    let (server, client) = connect_app(test_app(Some(hook), max_consecutive_drops)?).await?;
    Ok((server, client, events))
}

/// Send a malformed frame then a valid one, asserting the connection survives.
async fn continuing_case(policy: RecoveryPolicy) -> TestResult {
    let (server, mut client, mut events) = connect_with_hook(policy, 10).await?;
    let client_addr = client.local_addr()?;
    client.write_all(&malformed_frame()).await?;
    client.write_all(&envelope_frame(VALID_PAYLOAD)?).await?;
    let reply = read_reply(&mut client).await?;
    // Measured after the reply arrives so the pause spans the whole gap
    // between the failure callback and the next delivered frame.
    let received_at = Instant::now();
    drop(client);
    server.stop().await?;

    expect_payload(&reply_payload(&reply)?)?;
    let recorded = drain_events(&mut events);
    let event = exactly_one(&recorded)?;
    if event.policy != policy || event.error_type != "protocol" {
        return Err(
            format!("on_error should report the applied {policy:?} for a protocol error").into(),
        );
    }
    expect_peer(event, client_addr)?;
    let paused = received_at.duration_since(event.at);
    if policy == RecoveryPolicy::Quarantine && paused < QUARANTINE {
        return Err(format!(
            "the reply arrived after {paused:?}, before the {QUARANTINE:?} quarantine elapsed"
        )
        .into());
    }
    Ok(())
}

/// Drive `frames` malformed frames at a hook with `max_drops`, assert the
/// connection closes, and check the callbacks report `expected` from `peer`.
async fn closing_case(
    policy: RecoveryPolicy,
    max_drops: u32,
    frames: usize,
    expected: &[RecoveryPolicy],
) -> TestResult {
    let (server, mut client, mut events) = connect_with_hook(policy, max_drops).await?;
    let client_addr = client.local_addr()?;
    for _ in 0..frames {
        client.write_all(&malformed_frame()).await?;
    }
    expect_closed(&mut client).await?;
    drop(client);
    server.stop().await?;
    let recorded = expect_policies(&mut events, expected)?;
    expect_peer_on_all(&recorded, client_addr)
}

/// Connect with a disconnecting hook and assert the connection closes.
async fn disconnect_case() -> TestResult {
    closing_case(
        RecoveryPolicy::Disconnect,
        10,
        1,
        &[RecoveryPolicy::Disconnect],
    )
    .await
}

/// Connect without a hook and assert the default disconnect behaviour.
async fn no_hook_case() -> TestResult {
    let (server, mut client) = connect_app(test_app(None, 10)?).await?;
    client.write_all(&malformed_frame()).await?;
    expect_closed(&mut client).await?;
    drop(client);
    server.stop().await
}

/// Exhaust a one-frame drop budget and assert the second failure disconnects.
async fn drop_budget_case() -> TestResult {
    closing_case(
        RecoveryPolicy::Drop,
        1,
        2,
        &[RecoveryPolicy::Drop, RecoveryPolicy::Disconnect],
    )
    .await
}

/// Send a frame the built-in codec rejects before consuming it.
///
/// The decoder reports the over-long length prefix as a framing error rather
/// than an opaque I/O failure, and because it consumed no bytes the selected
/// `Drop` escalates to a disconnect.
async fn oversized_built_in_frame_case() -> TestResult {
    let (hook, mut events) = recording_hook(RecoveryPolicy::Drop);
    let (server, mut client) = connect_app(built_in_app(hook)?).await?;
    let client_addr = client.local_addr()?;
    client
        .write_all(&oversized_frame(BUILT_IN_MAX_FRAME_LENGTH))
        .await?;
    expect_closed(&mut client).await?;
    drop(client);
    server.stop().await?;

    let recorded = drain_events(&mut events);
    let event = exactly_one(&recorded)?;
    expect_peer(event, client_addr)?;
    if event.error_type == "framing" && event.policy == RecoveryPolicy::Disconnect {
        Ok(())
    } else {
        Err(format!(
            "an oversized frame should be a disconnected framing error, saw {} with {:?}",
            event.error_type, event.policy
        )
        .into())
    }
}

/// Close a connection mid-frame and assert the EOF failure disconnects.
///
/// The hook selects `Drop`, but an end-of-input failure can never be a
/// budgeted drop: the frame boundary is unrecoverable, so the policy escalates
/// to a disconnect.
async fn truncated_frame_case() -> TestResult {
    let (server, mut client, mut events) = connect_with_hook(RecoveryPolicy::Drop, 10).await?;
    let client_addr = client.local_addr()?;
    client.write_all(&truncated_frame()).await?;
    client.shutdown().await?;
    expect_closed(&mut client).await?;
    drop(client);
    server.stop().await?;

    let recorded = drain_events(&mut events);
    let event = exactly_one(&recorded)?;
    expect_peer(event, client_addr)?;
    if event.error_type == "eof" && event.policy == RecoveryPolicy::Disconnect {
        Ok(())
    } else {
        Err(format!(
            "a truncated frame should disconnect, saw {} with {:?}",
            event.error_type, event.policy
        )
        .into())
    }
}

/// Run `case` under the local recorder and assert its codec-error counts.
fn assert_case_metrics<F>(case: F, expected: &[(&str, &str, u64)]) -> TestResult
where
    F: Future<Output = TestResult>,
{
    let observability = capture(case)?;
    assert_only_codec_errors(&observability, expected)
}

#[test]
fn drop_policy_keeps_the_connection_alive() -> TestResult {
    assert_case_metrics(
        continuing_case(RecoveryPolicy::Drop),
        &[("protocol", "drop", 1)],
    )
}

#[test]
fn quarantine_policy_pauses_before_continuing() -> TestResult {
    assert_case_metrics(
        continuing_case(RecoveryPolicy::Quarantine),
        &[("protocol", "quarantine", 1)],
    )
}

#[test]
fn disconnect_policy_closes_the_connection() -> TestResult {
    assert_case_metrics(disconnect_case(), &[("protocol", "disconnect", 1)])
}

#[test]
fn missing_hook_disconnects_and_records_the_metric() -> TestResult {
    assert_case_metrics(no_hook_case(), &[("protocol", "disconnect", 1)])
}

#[test]
fn drop_budget_escalates_to_a_disconnect() -> TestResult {
    assert_case_metrics(
        drop_budget_case(),
        &[("protocol", "drop", 1), ("protocol", "disconnect", 1)],
    )
}

#[test]
fn truncated_frame_is_an_eof_failure() -> TestResult {
    assert_case_metrics(truncated_frame_case(), &[("eof", "disconnect", 1)])
}

#[test]
fn built_in_oversized_frame_is_a_framing_error() -> TestResult {
    assert_case_metrics(
        oversized_built_in_frame_case(),
        &[("framing", "disconnect", 1)],
    )
}
