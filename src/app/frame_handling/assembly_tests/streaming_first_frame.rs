//! Malformed first frames for a streaming route stay recoverable.
//!
//! A peer that sends a broken frame should cost the connection one recorded
//! deserialization failure, not an immediate disconnect. These tests pin that
//! contract for the streaming branch of first-frame dispatch, which builds its
//! own frame input rather than reusing the buffered admission helper.

use std::{collections::HashMap, io, sync::Arc};

use rstest::rstest;

use super::{
    AssemblyRuntime,
    FrameOutcome,
    assemble_if_needed,
    load_message_assembly_state,
    message_assembler,
    message_assembly_state,
    ok,
};
use crate::{
    app::{Envelope, StreamingHandler},
    message_assembler::{MessageAssemblyState, MessageKey},
    request::{RequestBodyStream, RequestParts},
    test_helpers,
};

/// Route identifier registered as a streaming route by these tests.
const STREAM_ROUTE: u32 = 9;

#[rstest]
fn malformed_streaming_first_frame_stays_pending_below_the_limit(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut state = Some(load_message_assembly_state(message_assembly_state));
    let routes = streaming_routes();

    let outcome = ok(
        assemble_if_needed(
            AssemblyRuntime::new(Some(&message_assembler), &mut state, &routes, false),
            &mut deser_failures,
            truncated_first_frame(),
            10,
        ),
        "a malformed frame below the limit should not end the connection",
    );

    assert!(
        matches!(outcome, FrameOutcome::Pending),
        "expected a pending outcome, got {outcome:?}"
    );
    assert_eq!(
        deser_failures, 1,
        "the malformed streaming frame should be counted"
    );
}

#[rstest]
fn malformed_streaming_first_frame_ends_the_connection_at_the_limit(
    message_assembler: Arc<dyn crate::message_assembler::MessageAssembler>,
    message_assembly_state: Result<MessageAssemblyState, &'static str>,
) {
    let mut deser_failures = 0_u32;
    let mut state = Some(load_message_assembly_state(message_assembly_state));
    let routes = streaming_routes();

    let error = assemble_if_needed(
        AssemblyRuntime::new(Some(&message_assembler), &mut state, &routes, false),
        &mut deser_failures,
        truncated_first_frame(),
        1,
    )
    .expect_err("exhausting the failure budget should end the connection");

    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "the connection should end for the recorded failure reason"
    );
}

/// Register the streaming route these frames are addressed to.
fn streaming_routes() -> HashMap<u32, StreamingHandler> {
    HashMap::from([(STREAM_ROUTE, unused_handler())])
}

/// Build a handler that these tests never run.
fn unused_handler() -> StreamingHandler {
    Arc::new(|_parts: RequestParts, _body: RequestBodyStream| Box::pin(async { None }))
}

/// Build a first frame that is one byte shorter than its own header declares.
fn truncated_first_frame() -> Envelope {
    let payload = ok(
        test_helpers::first_frame_payload(MessageKey::from(1), b"A1", false, Some(4)),
        "valid test payload",
    );
    let Some((_, truncated)) = payload.split_last() else {
        panic!("a first-frame payload should not be empty");
    };
    Envelope::new(STREAM_ROUTE, Some(7), truncated.to_vec())
}
