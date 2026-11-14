//! Correlation inheritance for streamed responses.
//!
//! Single, vector, and multi-packet responses inherit the request's correlation
//! identifier, so a client matching replies by identifier depends on streamed
//! frames and their protocol terminator behaving the same way. A request that
//! supplied no identifier must not clear an identifier the producer supplied.

use rstest::rstest;

use super::test_actor;
use crate::{
    app::Envelope,
    hooks::ProtocolHooks,
    response::{FrameStream, Response},
};

/// Route identity used by every frame in these tests.
const ROUTE: u32 = 4;
/// Route identity of the protocol terminator.
const TERMINATOR_ROUTE: u32 = 9;

/// Hooks supplying one protocol terminator.
fn terminating_hooks() -> ProtocolHooks<Envelope, ()> {
    ProtocolHooks {
        stream_end: Some(Box::new(|_ctx| {
            Some(Envelope::new(TERMINATOR_ROUTE, None, vec![]))
        })),
        ..ProtocolHooks::default()
    }
}

/// Drive a one-frame stream and return the frames that reached the sink.
async fn drive_stream(
    request_correlation: Option<u64>,
    frame_correlation: Option<u64>,
) -> Vec<Envelope> {
    let mut actor = test_actor(terminating_hooks()).expect("actor should build");
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![Ok(
        Envelope::new(ROUTE, frame_correlation, vec![1]),
    )]));
    let mut sink = Vec::new();

    actor
        .install_response(Response::Stream(stream), request_correlation, &mut sink)
        .await
        .expect("the stream should drive to completion");

    sink
}

/// A streamed frame and its terminator inherit the request identifier, while a
/// request without one leaves the producer's identifier alone.
#[rstest]
#[case::request_stamps_both(Some(7), None, Some(7), Some(7))]
#[case::request_outranks_the_frame(Some(7), Some(3), Some(7), Some(7))]
#[case::absent_request_keeps_the_frame_id(None, Some(3), Some(3), None)]
#[case::neither_supplies_an_id(None, None, None, None)]
#[tokio::test]
async fn streamed_frames_inherit_the_request_correlation(
    #[case] request_correlation: Option<u64>,
    #[case] frame_correlation: Option<u64>,
    #[case] expected_frame: Option<u64>,
    #[case] expected_terminator: Option<u64>,
) {
    let frames = drive_stream(request_correlation, frame_correlation).await;

    assert_eq!(frames.len(), 2, "the stream and its terminator must be written");
    let frame = frames.first().expect("the streamed frame should be present");
    assert_eq!(
        frame.correlation_id, expected_frame,
        "the streamed frame must carry the expected identifier"
    );
    assert_eq!(frame.id, ROUTE, "the frame identity must be preserved");
    let terminator = frames.get(1).expect("the terminator should be present");
    assert_eq!(
        terminator.correlation_id, expected_terminator,
        "the terminator must carry the expected identifier"
    );
    assert_eq!(
        terminator.id, TERMINATOR_ROUTE,
        "the terminator identity must be preserved"
    );
}