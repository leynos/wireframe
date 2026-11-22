//! Hook order for completed, clean, and rejected commands.
//!
//! These scenarios fix the documented dispatch order on a real connection:
//! setup before the first read; for a streamed response, one `before_send` per
//! frame, then `stream_end_frame`, the terminator's `before_send`, and
//! `on_command_end`. A clean close at a frame boundary runs no end-of-file
//! hook at all, and a malformed inbound frame is a decode failure rather than a
//! protocol error, so it reaches no hook.

use bytes::Bytes;
use rstest::rstest;
use tokio::io::AsyncWriteExt;
use wireframe_testing::{TestError, TestResult};

use super::harness::{
    DESER_FAILURE_LIMIT,
    HookScenario,
    ROUTE,
    TERMINATOR,
    build_request,
    decode_envelope,
    decode_unmarked_envelope,
    encode_payload,
    ensure,
    failing_handler,
    marked_body,
    next_frame,
    scenario,
    stream_handler,
};
use crate::hook_recorder::Event;

/// Every hook runs, in order, for a streamed response on a real connection.
#[rstest]
#[tokio::test]
async fn every_hook_runs_in_order_for_a_streamed_response(
    #[with(Some(Bytes::from_static(TERMINATOR)), stream_handler())]
    #[future]
    scenario: TestResult<HookScenario>,
) -> TestResult {
    let mut hook = scenario.await?;
    let mut frames = hook.connect_request().await?;
    let first = decode_envelope(&next_frame(&mut frames).await?)?;
    let second = decode_envelope(&next_frame(&mut frames).await?)?;
    let terminator = next_frame(&mut frames).await?;
    ensure(
        first.payload_bytes() == [1] && second.payload_bytes() == [2],
        "the streamed envelopes must arrive in order",
    )?;
    ensure(
        marked_body(&terminator)? == TERMINATOR,
        "the protocol terminator must arrive after the streamed frames",
    )?;

    // Dropping the client is a clean end-of-file at a frame boundary.
    drop(frames);
    hook.await_ended().await?;

    let events = hook.log.events();
    let [
        Event::Setup,
        Event::BeforeSend(first_frame),
        Event::BeforeSend(second_frame),
        Event::StreamEnd,
        Event::BeforeSend(end_frame),
        Event::CommandEnd,
    ] = events.as_slice()
    else {
        return Err(TestError::Msg(format!(
            "unexpected hook order for a streamed response: {events:?}"
        )));
    };
    ensure(
        decode_unmarked_envelope(first_frame)?.payload_bytes() == [1],
        "before_send must observe the first streamed frame",
    )?;
    ensure(
        decode_unmarked_envelope(second_frame)?.payload_bytes() == [2],
        "before_send must observe the second streamed frame",
    )?;
    ensure(
        end_frame.as_slice() == TERMINATOR,
        "before_send must observe the terminator the protocol returned",
    )?;

    hook.stop().await
}

/// A clean close at a frame boundary never reports a truncation.
#[rstest]
#[tokio::test]
async fn clean_close_does_not_run_the_eof_hook(
    #[future] scenario: TestResult<HookScenario>,
) -> TestResult {
    let mut hook = scenario.await?;
    let mut frames = hook.connect_request().await?;
    let reply = decode_envelope(&next_frame(&mut frames).await?)?;
    ensure(reply.payload_bytes() == [9], "the handler must reply")?;

    drop(frames);
    hook.await_ended().await?;

    let events = hook.log.events();
    ensure(
        !events.iter().any(|e| matches!(e, Event::Eof { .. })),
        format!("a clean close must not run on_eof: {events:?}"),
    )?;
    ensure(
        matches!(
            events.as_slice(),
            [Event::Setup, Event::BeforeSend(_), Event::CommandEnd]
        ),
        format!("a single response runs setup, before_send, then command end: {events:?}"),
    )?;

    hook.stop().await
}

/// A protocol error runs the error hook, then ends its command.
///
/// `handle_error` is reserved for a response yielding
/// `WireframeError::Protocol`, and the command that produced it still
/// completes, so a protocol that accounts for outcomes sees both callbacks
/// exactly once and in that order.
#[rstest]
#[tokio::test]
async fn a_protocol_error_runs_the_error_hook_then_command_end(
    #[with(None, failing_handler())]
    #[future]
    scenario: TestResult<HookScenario>,
) -> TestResult {
    let mut hook = scenario.await?;
    let mut client = hook.connect_raw().await?;
    client.write_all(&build_request(ROUTE)?).await?;
    // The request bytes precede the FIN, so the connection dispatches the
    // request before it observes the clean close.
    client.shutdown().await?;
    hook.await_ended().await?;

    let events = hook.log.events();
    ensure(
        events == vec![Event::Setup, Event::ProtocolError, Event::CommandEnd],
        format!("a protocol error runs the error hook, then command end: {events:?}"),
    )?;

    drop(client);
    hook.stop().await
}

/// Malformed inbound frames end the connection without protocol hooks.
///
/// One malformed frame is tolerated, so the peer must exceed the configured
/// limit before the connection closes. The close is a decode failure, not a
/// protocol error: `handle_error` is reserved for a handler returning
/// `WireframeError::Protocol`, and a decode failure is not a truncation either,
/// so `on_eof` must not run. The protocol observes nothing beyond setup.
#[rstest]
#[tokio::test]
async fn malformed_frames_end_the_connection_without_protocol_hooks(
    #[future] scenario: TestResult<HookScenario>,
) -> TestResult {
    let mut hook = scenario.await?;
    let mut client = hook.connect_raw().await?;
    let mut wire = Vec::new();
    for _ in 0..DESER_FAILURE_LIMIT {
        wire.extend_from_slice(&encode_payload(b"not an envelope")?);
    }
    client.write_all(&wire).await?;
    hook.await_ended().await?;

    let events = hook.log.events();
    ensure(
        events == vec![Event::Setup],
        format!("a decode failure must not reach protocol hooks: {events:?}"),
    )?;

    drop(client);
    hook.stop().await
}
