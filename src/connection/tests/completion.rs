//! Completion-hook ordering tests for the connection actor.
//!
//! A streamed or multi-packet response announces completion while its
//! terminator frame is still buffered. These tests pin the ordering the
//! prepared server path relies on: the protocol completion hook runs only
//! after the terminator has been written successfully.

use std::{
    future::Future,
    io,
    sync::{Arc, Mutex, PoisonError},
};

use rstest::rstest;
use tokio::sync::mpsc;

use super::test_actor;
use crate::{
    app::Envelope,
    connection::FrameSink,
    hooks::ProtocolHooks,
    response::{FrameStream, Response},
};

/// Sink that records each successful write in a shared event log.
struct RecordingSink {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl FrameSink<Envelope> for RecordingSink {
    // Recording cannot block, so the future is returned ready rather than
    // declared `async`; a needless `async` is rejected by the project lints.
    fn send_frame(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("write");
        std::future::ready(Ok(()))
    }
}

/// Sink that rejects every write, modelling a lost peer.
struct FailingSink;

impl FrameSink<Envelope> for FailingSink {
    // The failure is immediate, so no `async` state machine is needed.
    fn send_frame(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        std::future::ready(Err(io::Error::other("peer lost")))
    }
}

/// Hooks recording the completion hook alongside a supplied terminator.
fn ordering_hooks(events: Arc<Mutex<Vec<&'static str>>>) -> ProtocolHooks<Envelope, ()> {
    ProtocolHooks {
        on_command_end: Some(Box::new(move |_ctx| {
            events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push("hook");
        })),
        stream_end: Some(Box::new(|_ctx| Some(Envelope::new(9, None, vec![])))),
        ..ProtocolHooks::default()
    }
}

fn recorded_events(events: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Build a response that yields one frame and then drains.
async fn draining_response(is_streaming: bool) -> io::Result<Response<Envelope, ()>> {
    if is_streaming {
        let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![Ok(
            Envelope::new(1, None, vec![1]),
        )]));
        return Ok(Response::Stream(stream));
    }
    let (sender, receiver) = mpsc::channel(4);
    sender
        .send(Envelope::new(1, None, vec![1]))
        .await
        .map_err(|error| io::Error::other(format!("channel should accept a frame: {error}")))?;
    drop(sender);
    Ok(Response::MultiPacket(receiver))
}

/// Streams and drained channels both finish writing before the hook runs.
#[rstest]
#[case::stream(true)]
#[case::multi_packet(false)]
#[tokio::test]
async fn completion_hook_runs_after_the_terminator_is_written(#[case] is_streaming: bool) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut actor = test_actor(ordering_hooks(Arc::clone(&events))).expect("actor should build");
    let mut sink = RecordingSink {
        events: Arc::clone(&events),
    };

    actor
        .install_response(
            draining_response(is_streaming)
                .await
                .expect("channel should accept a frame"),
            None,
            &mut sink,
        )
        .await
        .expect("response should drive to completion");

    assert_eq!(
        recorded_events(&events),
        vec!["write", "write", "hook"],
        "the completion hook must run only after the terminator write"
    );
}

#[tokio::test]
async fn completion_hook_is_skipped_when_the_terminator_write_fails() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut actor = test_actor(ordering_hooks(Arc::clone(&events))).expect("actor should build");
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::empty());
    let mut sink = FailingSink;

    actor
        .install_response(Response::Stream(stream), None, &mut sink)
        .await
        .expect_err("a failed terminator write must be reported");

    assert!(
        recorded_events(&events).is_empty(),
        "a failed write must not report command completion"
    );
}
