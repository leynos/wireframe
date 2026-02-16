//! Tests for extracting shared state from message requests.
//!
//! They verify successful extraction and error handling when state is missing.
#![cfg(not(loom))]

use rstest::{fixture, rstest};
use wireframe::extractor::{
    ExtractError,
    FromMessageRequest,
    MessageRequest,
    Payload,
    SharedState,
};

#[fixture]
fn request() -> MessageRequest {
    // Default request for shared state extraction tests
    MessageRequest::default()
}

#[fixture]
fn empty_payload() -> Payload<'static> {
    // Empty payload for shared state extraction tests
    Payload::default()
}

#[rstest]
fn shared_state_extractor_returns_data(
    request: MessageRequest,
    mut empty_payload: Payload<'static>,
) {
    request.insert_state(5u32);
    let extracted = SharedState::<u32>::from_message_request(&request, &mut empty_payload)
        .expect("failed to extract shared state");
    assert_eq!(*extracted, 5);
}

#[rstest]
fn missing_shared_state_returns_error(
    request: MessageRequest,
    mut empty_payload: Payload<'static>,
) {
    let err = SharedState::<u32>::from_message_request(&request, &mut empty_payload)
        .expect_err("missing state error expected");
    assert!(matches!(err, ExtractError::MissingState(_)));
}
