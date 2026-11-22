//! Server-initiated pushes on a real connection.
//!
//! A protocol that captures the push handle in `on_connection_setup` can send
//! frames at any time. Pushes must be written while a handler is still running,
//! through the same `before_send` path as a response, and a handle retained past
//! the connection must report closure rather than silently discarding frames.

use std::sync::Arc;

use bytes::Bytes;
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_util::codec::Framed;
use wireframe::{
    codec::FrameCodec,
    push::{PushError, PushPolicy, PushPriority},
};
use wireframe_testing::{TestError, TestResult};

use super::harness::{
    ROUTE,
    await_connection_end,
    build_app,
    build_request,
    client_codec,
    decode_envelope,
    decode_unmarked_envelope,
    ensure,
    marked_body,
    next_frame,
    push_then_reply,
    single_handler,
    start_server,
};
use crate::hook_recorder::{Event, HookLog};

/// A handler's pushes drain while it is still running.
///
/// The push queues hold one frame per priority, so the handler's second push
/// only completes once the connection has dequeued the first: a connection
/// that waited for the handler before writing anything would deadlock. The
/// reply is then delivered as one batch, so a push still queued when the
/// handler finishes may trail it rather than precede it.
#[tokio::test]
async fn handler_pushes_drain_while_it_is_pending() -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    let app = build_app(&log, None, ended_tx, push_then_reply(Arc::clone(&log)))?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    client.write_all(&build_request(ROUTE)?).await?;
    let mut frames = Framed::new(client, client_codec().decoder());

    let first = next_frame(&mut frames).await?;
    let second = next_frame(&mut frames).await?;
    let third = next_frame(&mut frames).await?;
    ensure(
        marked_body(&first)? == b"PUSH-1",
        "the first push must be written while the handler is pending",
    )?;
    let (second_pushes, replies) = count_trailing_frames([&second, &third])?;
    ensure(
        second_pushes == 1 && replies == 1,
        format!("the second push and the reply must both arrive: {second:?}, {third:?}"),
    )?;

    drop(frames);
    await_connection_end(ended_rx).await?;
    let events = log.events();
    ensure(
        events.first() == Some(&Event::Setup),
        format!("setup must run once, before any frame is written: {events:?}"),
    )?;
    let tally = tally_push_events(&events)?;
    ensure(
        tally.setups == 1 && tally.pushes == 2,
        format!("setup and each push run their hooks exactly once: {events:?}"),
    )?;
    ensure(
        matches!(
            (tally.reply, tally.command_end),
            (Some(reply_at), Some(end)) if reply_at < end
        ),
        format!("completion follows the reply it belongs to: {events:?}"),
    )?;

    server.stop().await
}

/// Count the push and reply frames among the two frames that may follow the
/// first push, in either order.
fn count_trailing_frames(frames: [&Bytes; 2]) -> TestResult<(u8, u8)> {
    let mut second_pushes = 0u8;
    let mut replies = 0u8;
    for frame in frames {
        let body = marked_body(frame)?;
        if body == b"PUSH-2" {
            second_pushes += 1;
        } else if decode_unmarked_envelope(&body)?.payload_bytes() == [9] {
            replies += 1;
        }
    }
    Ok((second_pushes, replies))
}

/// Hook events a push-only command may produce, and where they appeared.
#[derive(Default)]
struct PushEventTally {
    setups: u8,
    pushes: u8,
    reply: Option<usize>,
    command_end: Option<usize>,
}

impl PushEventTally {
    /// Record one event, rejecting anything a push-only command cannot emit.
    fn record(&mut self, index: usize, event: &Event) -> TestResult {
        match event {
            Event::Setup => self.setups += 1,
            Event::BeforeSend(frame) if is_push_frame(frame) => self.pushes += 1,
            Event::BeforeSend(frame) => {
                ensure(
                    decode_unmarked_envelope(frame)?.payload_bytes() == [9],
                    format!("before_send observed an unexpected frame: {frame:?}"),
                )?;
                self.reply = Some(index);
            }
            Event::CommandEnd => {
                // A second completion for one command would overwrite the
                // first index and hide a duplicated hook call.
                if self.command_end.is_some() {
                    return Err("completion must run at most once per command".into());
                }
                self.command_end = Some(index);
            }
            other => {
                return Err(TestError::Msg(format!(
                    "a push-only command must run no other hook: {other:?}"
                )));
            }
        }
        Ok(())
    }
}

/// Whether a hook-observed frame is one of the handler's pushes.
fn is_push_frame(frame: &[u8]) -> bool { matches!(frame, b"PUSH-1" | b"PUSH-2") }

/// Classify the whole hook log of the push-only command.
fn tally_push_events(events: &[Event]) -> TestResult<PushEventTally> {
    let mut tally = PushEventTally::default();
    for (index, event) in events.iter().enumerate() {
        tally.record(index, event)?;
    }
    Ok(tally)
}

/// A push handle retained past the connection reports closure.
#[tokio::test]
async fn retained_push_handle_reports_closed_after_the_connection_ends() -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    let app = build_app(&log, None, ended_tx, single_handler())?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    client.write_all(&build_request(ROUTE)?).await?;
    let mut frames = Framed::new(client, client_codec().decoder());
    let _reply = decode_envelope(&next_frame(&mut frames).await?)?;
    drop(frames);
    await_connection_end(ended_rx).await?;

    let handle = log
        .push_handle()
        .ok_or_else(|| TestError::Msg("connection setup must capture a handle".into()))?;
    let outcome = handle.try_push(
        Bytes::from_static(b"late"),
        PushPriority::High,
        PushPolicy::ReturnErrorIfFull,
    );
    ensure(
        matches!(outcome, Err(PushError::Closed)),
        format!("a handle retained past the connection must see closure: {outcome:?}"),
    )?;

    server.stop().await
}
