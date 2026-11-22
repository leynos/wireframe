//! A push queued during a streamed response is delivered without disturbing
//! the response.
//!
//! The connection actor interleaves queued pushes with a response stream, so
//! the position of a push among the streamed frames is a scheduling decision
//! rather than a contract. What the contract does fix is each producer's own
//! order and its hooks: the streamed envelopes stay in order, the push passes
//! through `before_send` exactly once, and the response still ends with
//! `stream_end_frame` and one `on_command_end`. This scenario fails if a
//! streamed frame is reordered, lost, or written without its hook, or if the
//! push skips `before_send`.

use std::sync::Arc;

use bytes::Bytes;
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_util::codec::Framed;
use wireframe::{
    WireframeError,
    app::{Envelope, ResponseHandler},
    codec::FrameCodec,
    response::{FrameStream, Response},
};
use wireframe_testing::{TestError, TestResult};

use super::harness::{
    ROUTE,
    await_connection_end,
    build_app,
    build_request,
    client_codec,
    decode_unmarked_envelope,
    ensure,
    marked_body,
    next_frame,
    start_server,
};
use crate::hook_recorder::{Event, HookLog};

/// Frames the handler's stream yields before it ends.
const STREAMED_FRAMES: u8 = 3;
/// Stream index at which the push is queued, so one frame is already written.
const PUSH_QUEUE_AT: u8 = 1;
/// Push payload the handler queues part-way through its stream.
const MID_STREAM_PUSH: &[u8] = b"MID-STREAM";

/// Handler whose stream queues a push between its own frames.
///
/// The push is submitted from inside the stream, so it becomes ready while the
/// response is still being delivered rather than before the run starts.
fn stream_that_pushes(log: &Arc<HookLog>, push_frame: Bytes) -> ResponseHandler<Envelope> {
    let hook_log = Arc::clone(log);
    Arc::new(move |_request: &Envelope| {
        let handler_log = Arc::clone(&hook_log);
        let handler_push = push_frame.clone();
        Box::pin(async move {
            let frames = futures::stream::unfold(Some(0_u8), move |state| {
                let step_log = Arc::clone(&handler_log);
                let step_push = handler_push.clone();
                stream_step(state, step_log, step_push)
            });
            let frame_stream: FrameStream<Envelope, ()> = Box::pin(frames);
            Response::Stream(frame_stream)
        })
    })
}

/// One step of the handler's stream: the frame to send and the next state.
type StreamStep = Option<(Result<Envelope, WireframeError<()>>, Option<u8>)>;

/// Yield the frame for `state`, queueing the push once at its index.
///
/// The step is a named function rather than an inline `async` block so the
/// queueing guard and the frame construction each stay one nesting level deep.
async fn stream_step(state: Option<u8>, log: Arc<HookLog>, push_frame: Bytes) -> StreamStep {
    let index = state?;
    if index == PUSH_QUEUE_AT {
        queue_mid_stream_push(log.as_ref(), push_frame).await;
    }
    let next = (index + 1 < STREAMED_FRAMES).then_some(index + 1);
    Some((Ok(Envelope::new(ROUTE, None, vec![index + 1])), next))
}

/// Submit the push the scenario expects to follow the whole response batch.
async fn queue_mid_stream_push(log: &HookLog, push_frame: Bytes) {
    let handle = log
        .push_handle()
        .expect("connection setup must capture the push handle");
    handle
        .push_high_priority(push_frame)
        .await
        .expect("the push queue must accept one frame");
}

/// What one received wire frame turned out to be.
enum Received {
    /// One of the response's streamed envelopes, with its payload.
    Streamed(Vec<u8>),
    /// The push the handler queued, with its payload.
    Pushed(Vec<u8>),
}

/// Payloads of the response's streamed envelopes, in arrival order.
type StreamedPayloads = Vec<Vec<u8>>;
/// The push the handler queued, when the wire carried it.
type PushedPayload = Option<Vec<u8>>;

/// Classify every received frame, keeping each producer's order.
fn classify(wire: Vec<Bytes>) -> TestResult<(StreamedPayloads, PushedPayload)> {
    let mut streamed = Vec::new();
    let mut pushed = None;
    for frame in wire {
        match classify_frame(&frame)? {
            Received::Streamed(payload) => streamed.push(payload),
            Received::Pushed(payload) => {
                if pushed.replace(payload).is_some() {
                    return Err("the push was written more than once".into());
                }
            }
        }
    }
    Ok((streamed, pushed))
}

/// Decide whether one marked frame is a streamed envelope or the push.
fn classify_frame(frame: &Bytes) -> TestResult<Received> {
    let body = marked_body(frame)?;
    if body == MID_STREAM_PUSH {
        return Ok(Received::Pushed(body));
    }
    let envelope = decode_unmarked_envelope(&body)?;
    Ok(Received::Streamed(envelope.payload_bytes().to_vec()))
}

/// Index of the first event matching `predicate`.
fn position(events: &[Event], predicate: impl Fn(&Event) -> bool) -> Option<usize> {
    events.iter().position(predicate)
}

/// Split the `before_send` observations into streamed payloads and push count.
///
/// Every frame is one producer's, so the split recovers what the hook saw from
/// each without assuming anything about their relative order.
fn observed_before_send(events: &[Event]) -> TestResult<(StreamedPayloads, usize)> {
    let mut streamed = Vec::new();
    let mut pushes = 0;
    for event in events {
        let Event::BeforeSend(frame) = event else {
            continue;
        };
        if frame.as_slice() == MID_STREAM_PUSH {
            pushes += 1;
        } else {
            streamed.push(decode_unmarked_envelope(frame)?.payload_bytes().to_vec());
        }
    }
    Ok((streamed, pushes))
}

/// Require exactly one terminator and one completion, in that order.
fn assert_terminator_order(events: &[Event]) -> TestResult {
    ensure(
        events
            .iter()
            .filter(|event| matches!(event, Event::StreamEnd))
            .count()
            == 1,
        format!("the response must end with exactly one terminator: {events:?}"),
    )?;
    ensure(
        events
            .iter()
            .filter(|event| matches!(event, Event::CommandEnd))
            .count()
            == 1,
        format!("the command must complete exactly once: {events:?}"),
    )?;
    let stream_end =
        position(events, |event| matches!(event, Event::StreamEnd)).ok_or_else(|| {
            TestError::Msg(format!(
                "the response must end with its terminator: {events:?}"
            ))
        })?;
    let before_end = events.get(..stream_end).ok_or_else(|| {
        TestError::Msg(format!("the terminator index must be in range: {events:?}"))
    })?;
    let streamed_before_end = before_end
        .iter()
        .filter(|event| matches!(event, Event::BeforeSend(frame) if frame.as_slice() != MID_STREAM_PUSH))
        .count();
    ensure(
        streamed_before_end == usize::from(STREAMED_FRAMES),
        format!("the terminator must follow every streamed frame: {events:?}"),
    )?;
    let command_end = position(events, |event| matches!(event, Event::CommandEnd))
        .ok_or_else(|| TestError::Msg(format!("the command must complete: {events:?}")))?;
    ensure(
        stream_end < command_end,
        format!("the response must end with its terminator, then completion: {events:?}"),
    )
}

/// A push queued mid-stream is delivered, and neither producer is disturbed.
#[tokio::test]
async fn a_push_queued_during_a_stream_is_delivered_once() -> TestResult {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    // Pushes carry the frame payload, which the connection encodes for the
    // wire; encoding it here would nest a second length prefix.
    let push_frame = Bytes::from_static(MID_STREAM_PUSH);
    let app = build_app(&log, None, ended_tx, stream_that_pushes(&log, push_frame))?;
    let server = start_server(app).await?;

    let mut client = TcpStream::connect(server.addr).await?;
    client.write_all(&build_request(ROUTE)?).await?;
    let mut frames = Framed::new(client, client_codec().decoder());

    // One frame per streamed envelope, plus the push, in arrival order.
    let mut wire = Vec::new();
    for _ in 0..=STREAMED_FRAMES {
        wire.push(next_frame(&mut frames).await?);
    }
    let (streamed, pushed) = classify(wire)?;
    ensure(
        streamed == vec![vec![1], vec![2], vec![3]],
        format!("the streamed envelopes must arrive in order: {streamed:?}"),
    )?;
    ensure(
        pushed.as_deref() == Some(MID_STREAM_PUSH),
        format!("the push must arrive exactly once: {pushed:?}"),
    )?;

    drop(frames);
    await_connection_end(ended_rx).await?;

    let events = log.events();
    ensure(
        events.first() == Some(&Event::Setup),
        format!("setup must run once, before any frame is written: {events:?}"),
    )?;
    let (observed_streams, observed_pushes) = observed_before_send(&events)?;
    ensure(
        observed_streams == vec![vec![1], vec![2], vec![3]],
        format!("before_send must observe every streamed frame in order: {events:?}"),
    )?;
    ensure(
        observed_pushes == 1,
        format!("the push must pass through before_send exactly once: {events:?}"),
    )?;
    assert_terminator_order(&events)?;

    server.stop().await
}
