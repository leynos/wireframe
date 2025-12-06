#![cfg(not(loom))]
//! Tests for message request extractors.
//!
//! Validate message parsing, connection info, and shared state behaviour.

use std::net::SocketAddr;

use rstest::{fixture, rstest};
use wireframe::{
    extractor::{ConnectionInfo, FromMessageRequest, Message, MessageRequest, Payload},
    message::Message as MessageTrait,
};

#[fixture]
fn request() -> MessageRequest {
    // default request used across extractor tests
    MessageRequest::default()
}

#[fixture]
fn empty_payload() -> Payload<'static> {
    // simple empty payload ensures extractors handle zero-length bodies
    Payload::default()
}

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestMsg(u8);

/// Tests that a message can be extracted from a payload and that the payload cursor advances fully.
///
/// Verifies that a `TestMsg` instance serialised into bytes can be correctly extracted from a
/// `Payload` using `Message::<TestMsg>::from_message_request`, and asserts that the payload has no
/// remaining unread data after extraction.
#[rstest]
fn message_extractor_parses_and_advances(request: MessageRequest) {
    let msg = TestMsg(42);
    let bytes = msg.to_bytes().expect("failed to serialise message");
    let mut payload = Payload::new(bytes.as_slice());

    let extracted = Message::<TestMsg>::from_message_request(&request, &mut payload)
        .expect("failed to extract TestMsg from payload");
    assert_eq!(*extracted, msg);
    assert_eq!(payload.remaining(), 0);
}

#[rstest]
/// Tests that `ConnectionInfo` correctly reports the peer socket address extracted from a
/// `MessageRequest`.
fn connection_info_reports_peer(mut request: MessageRequest, mut empty_payload: Payload<'static>) {
    let addr: SocketAddr = "127.0.0.1:12345"
        .parse()
        .expect("hard-coded socket address must be valid");
    request.peer_addr = Some(addr);
    let info = ConnectionInfo::from_message_request(&request, &mut empty_payload)
        .expect("failed to build ConnectionInfo");
    assert_eq!(info.peer_addr(), Some(addr));
}

/// Tests that shared state of type `u8` can be successfully extracted from a `MessageRequest`'s
/// `app_data`.
///
/// Inserts an `Arc<u8>` into the request's shared state, extracts it using the `SharedState`
/// extractor, and asserts that the extracted value matches the original.
#[rstest]
fn shared_state_extractor(mut request: MessageRequest, mut empty_payload: Payload<'static>) {
    request.insert_state(42u8);

    let state =
        wireframe::extractor::SharedState::<u8>::from_message_request(&request, &mut empty_payload)
            .expect("failed to extract shared state");
    assert_eq!(*state, 42);
}

/// Tests that extracting a missing shared state from a `MessageRequest`
/// returns an `ExtractError::MissingState` containing the type name.
///
/// Ensures that when no shared state of the requested type is present,
/// the correct error is produced and includes the expected type information.
#[rstest]
fn shared_state_missing_error(request: MessageRequest, mut empty_payload: Payload<'static>) {
    let Err(err) =
        wireframe::extractor::SharedState::<u8>::from_message_request(&request, &mut empty_payload)
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
