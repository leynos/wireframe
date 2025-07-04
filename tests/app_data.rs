//! Tests for extracting shared state from message requests.
//!
//! They verify successful extraction and error handling when state is missing.

use wireframe::extractor::{
    ExtractError,
    FromMessageRequest,
    MessageRequest,
    Payload,
    SharedState,
};

#[test]
fn shared_state_extractor_returns_data() {
    let mut req = MessageRequest::default();
    req.insert_state(5u32);
    let mut payload = Payload::default();
    let extracted = SharedState::<u32>::from_message_request(&req, &mut payload).unwrap();
    assert_eq!(*extracted, 5);
}

#[test]
fn missing_shared_state_returns_error() {
    let req = MessageRequest::default();
    let mut payload = Payload::default();
    let err = SharedState::<u32>::from_message_request(&req, &mut payload)
        .err()
        .unwrap();
    assert!(matches!(err, ExtractError::MissingState(_)));
}
