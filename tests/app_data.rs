//! Tests for extracting shared state from message requests.
//!
//! They verify successful extraction and error handling when state is missing.

use rstest::{fixture, rstest};
use wireframe::extractor::{
    ExtractError,
    FromMessageRequest,
    MessageRequest,
    Payload,
    SharedState,
};

#[allow(unused_braces)]
#[fixture]
fn request() -> MessageRequest { MessageRequest::default() }

#[allow(unused_braces)]
#[fixture]
fn empty_payload() -> Payload<'static> { Payload::default() }

#[rstest]
fn shared_state_extractor_returns_data(
    mut request: MessageRequest,
    mut empty_payload: Payload<'static>,
) {
    request.insert_state(5u32);
    let extracted = SharedState::<u32>::from_message_request(&request, &mut empty_payload).unwrap();
    assert_eq!(*extracted, 5);
}

#[rstest]
fn missing_shared_state_returns_error(
    request: MessageRequest,
    mut empty_payload: Payload<'static>,
) {
    let err = SharedState::<u32>::from_message_request(&request, &mut empty_payload)
        .err()
        .unwrap();
    assert!(matches!(err, ExtractError::MissingState(_)));
}
