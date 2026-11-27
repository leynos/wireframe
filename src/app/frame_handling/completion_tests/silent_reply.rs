//! Completion hook for a streaming handler that chooses not to answer.
//!
//! A streaming handler may return no reply at all. Nothing is written for it,
//! so there is no terminator and no delivery span, but the request it served is
//! still finished. The completion hook therefore runs exactly as it does for a
//! buffered [`Response::Empty`], which writes no frame either. These cases pin
//! that the hook runs once for silence and that silence writes nothing.

use std::sync::Arc;

use super::*;

/// Run an absent streaming reply through the forwarded-reply path.
///
/// # Errors
///
/// Returns the push-queue configuration failure. The dispatch decision is
/// returned alongside the log so a failing scenario can still inspect it.
async fn forward_silent_reply() -> Result<(EventLog, io::Result<Dispatch>), PushConfigError> {
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let (mut actor, _handle) = build_actor()?;
    let mut sink = RecordingSink {
        log: Arc::clone(&log),
        fail: false,
    };
    let codec = LengthDelimitedFrameCodec::new(1024);
    let serializer = BincodeSerializer;
    let protocol: Arc<dyn WireframeProtocol<Frame = Bytes, ProtocolError = ()>> =
        Arc::new(RecordingProtocol {
            log: Arc::clone(&log),
        });
    // The pipeline owns the connection's callbacks, so the completion hook the
    // forwarded reply runs is the one installed here.
    let mut pipeline: FramePipeline<Bytes> =
        FramePipeline::with_hooks(None, ProtocolHooks::from_protocol(&protocol));

    let outcome = super::super::forward_service_response(
        7,
        Some(3),
        None,
        ResponseContext {
            serializer: &serializer,
            sink: &mut sink,
            pipeline: &mut pipeline,
            codec: &codec,
            actor: &mut actor,
        },
    )
    .await;
    Ok((log, outcome))
}

/// A handler that stays silent still runs the completion hook, exactly once.
#[tokio::test]
async fn a_silent_streaming_reply_runs_the_completion_hook_once() {
    let (log, outcome) = forward_silent_reply()
        .await
        .expect("the actor should build");

    assert_eq!(
        outcome.expect("silence is not a transport error"),
        Dispatch::Continue,
        "a silent handler lets the loop dispatch the next frame"
    );
    assert_eq!(
        events(&log),
        vec!["command_end"],
        "the hook must run once, and silence must write no frame"
    );
}
