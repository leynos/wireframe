//! Tests for message request extractors.
//!
//! Validate message parsing, connection info, and shared state behaviour.

use std::net::SocketAddr;

use wireframe::{
    extractor::{ConnectionInfo, FromMessageRequest, Message, MessageRequest, Payload},
    message::Message as MessageTrait,
};

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestMsg(u8);

#[test]
/// Tests that a message can be extracted from a payload and that the payload cursor advances fully.
///
/// Verifies that a `TestMsg` instance serialised into bytes can be correctly extracted from a
/// `Payload` using `Message::<TestMsg>::from_message_request`, and asserts that the payload has no
/// remaining unread data after extraction.
fn message_extractor_parses_and_advances() {
    let msg = TestMsg(42);
    let bytes = msg.to_bytes().unwrap();
    let mut payload = Payload::new(bytes.as_slice());
    let req = MessageRequest::default();

    let extracted = Message::<TestMsg>::from_message_request(&req, &mut payload).unwrap();
    assert_eq!(*extracted, msg);
    assert_eq!(payload.remaining(), 0);
}

#[test]
/// Tests that `ConnectionInfo` correctly reports the peer socket address extracted from a
/// `MessageRequest`.
fn connection_info_reports_peer() {
    let addr: SocketAddr = "127.0.0.1:12345"
        .parse()
        .expect("hard-coded socket address must be valid");
    let req = MessageRequest {
        peer_addr: Some(addr),
        ..Default::default()
    };
    let mut payload = Payload::default();
    let info = ConnectionInfo::from_message_request(&req, &mut payload).unwrap();
    assert_eq!(info.peer_addr(), Some(addr));
}

#[test]
/// Tests that shared state of type `u8` can be successfully extracted from a `MessageRequest`'s
/// `app_data`.
///
/// Inserts an `Arc<u8>` into the request's shared state, extracts it using the `SharedState`
/// extractor, and asserts that the extracted value matches the original.
fn shared_state_extractor() {
    let mut req = MessageRequest::default();
    req.insert_state(42u8);
    let mut payload = Payload::default();

    let state =
        wireframe::extractor::SharedState::<u8>::from_message_request(&req, &mut payload).unwrap();
    assert_eq!(*state, 42);
}

#[test]
/// Tests that extracting a missing shared state from a `MessageRequest`
/// returns an `ExtractError::MissingState` containing the type name.
///
/// Ensures that when no shared state of the requested type is present,
/// the correct error is produced and includes the expected type information.
fn shared_state_missing_error() {
    let req = MessageRequest::default();
    let mut payload = Payload::default();
    let Err(err) =
        wireframe::extractor::SharedState::<u8>::from_message_request(&req, &mut payload)
    else {
        panic!("expected error");
    };
    match err {
        wireframe::extractor::ExtractError::MissingState(name) => {
            assert!(name.contains("u8"));
        }
        _ => panic!("unexpected error"),
    }
}
