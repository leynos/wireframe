//! Draining a response across shutdown, and abandoning it on request.
//!
//! ADR-014 requires a server connection to keep draining a response whose peer
//! is still reading after `stop()`, so the stop token bounds only writes that
//! cannot progress. The actor therefore lets its owner choose whether a
//! cancelled shutdown token ends the response drive, and the direct actor API
//! keeps the abandon behaviour its own tests pin.

use std::{future::Future, io};

use tokio_util::sync::CancellationToken;

use super::test_actor_with_shutdown;
use crate::{
    app::Envelope,
    connection::{FrameSink, ResponseOutcome},
    hooks::ProtocolHooks,
    response::{FrameStream, Response},
};

/// Sink that records each payload byte and can request shutdown after the
/// first frame.
///
/// Cancelling the token inside the write models a server `stop()` that arrives
/// while the response is still producing, without depending on scheduling.
struct RecordingSink {
    /// Token cancelled once the first frame has been accepted, when set.
    shutdown: Option<CancellationToken>,
    /// Payload byte of every accepted frame, in order.
    accepted: Vec<u8>,
}

impl FrameSink<Envelope> for RecordingSink {
    fn send_frame(&mut self, frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        if let Some(byte) = frame.payload_bytes().first() {
            self.accepted.push(*byte);
        }
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.cancel();
        }
        std::future::ready(Ok(()))
    }
}

/// A three-frame response whose payload byte numbers each frame.
fn three_frame_response() -> Response<Envelope, ()> {
    let frames = (1..=3)
        .map(|number| Ok(Envelope::new(1, None, vec![number])))
        .collect::<Vec<_>>();
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(frames));
    Response::Stream(stream)
}

/// Shutdown either drains the response or abandons it, as the owner chose.
///
/// The stop request arrives after the first frame reached the sink, while the
/// stream still has two frames to produce. A connection that drains writes all
/// three and reports completion; the direct actor API stops after the frame it
/// had already written.
#[rstest::rstest]
#[case::drains_across_shutdown(false, 3, ResponseOutcome::Completed)]
#[case::abandons_on_shutdown(true, 1, ResponseOutcome::Cancelled)]
#[tokio::test]
async fn shutdown_either_drains_or_abandons_a_stream(
    #[case] abandon_on_shutdown: bool,
    #[case] expected_frames: usize,
    #[case] expected_outcome: ResponseOutcome,
) {
    let shutdown = CancellationToken::new();
    let mut actor = test_actor_with_shutdown(shutdown.clone(), ProtocolHooks::default())
        .expect("the actor should build");
    actor.set_abandon_response_on_shutdown(abandon_on_shutdown);
    let mut sink = RecordingSink {
        shutdown: Some(shutdown),
        accepted: Vec::new(),
    };

    let outcome = actor
        .install_response(three_frame_response(), Some(7), &mut sink)
        .await
        .expect("a bounded response is not an error");

    assert_eq!(outcome, expected_outcome);
    assert_eq!(
        sink.accepted.len(),
        expected_frames,
        "unexpected frames reached the sink: {:?}",
        sink.accepted
    );
}
