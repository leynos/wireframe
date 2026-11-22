//! Actor fixtures shared by the hook-dispatch test binaries.
//!
//! The example-based dispatch tests and their property sibling drive the same
//! kind of connection: prefilled push queues, an optional response stream, and
//! a recording protocol. Keeping the construction and failure-mapping helpers
//! here stops the two binaries drifting apart, and keeps each one inside the
//! repository's file-length limit. Scenario-specific values, such as the
//! terminator frame, stay with the binary that uses them. Include this module
//! with `#[path = "common/dispatch_fixtures.rs"] mod dispatch_fixtures;`.

use std::io;

use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::{ConnectionActor, ConnectionChannels},
    hooks::ProtocolHooks,
    push::{PushHandle, PushQueues},
    response::{FrameStream, WireframeError},
};
use wireframe_testing::TestResult;

/// Push queues paired with the handle a test uses to prefill them.
pub type TestQueues = (PushQueues<Vec<u8>>, PushHandle<Vec<u8>>);

/// Build push queues sized for the frames each test prefills.
pub fn queues() -> TestResult<TestQueues> {
    Ok(PushQueues::<Vec<u8>>::builder()
        .high_capacity(16)
        .low_capacity(16)
        .unlimited()
        .build()?)
}

/// Build an actor over prefilled queues and the supplied hooks.
pub fn actor_with_hooks(
    queues: PushQueues<Vec<u8>>,
    handle: PushHandle<Vec<u8>>,
    response: Option<FrameStream<Vec<u8>, ()>>,
    hooks: ProtocolHooks<Vec<u8>, ()>,
) -> ConnectionActor<Vec<u8>, ()> {
    ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        response,
        CancellationToken::new(),
        hooks,
    )
}

/// Present an actor error as a test failure without losing its debug form.
pub fn run_error(error: &WireframeError<()>) -> io::Error {
    io::Error::other(format!("actor run failed: {error:?}"))
}
