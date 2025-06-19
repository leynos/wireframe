use std::{collections::HashMap, net::SocketAddr};

use wireframe::{
    extractor::{ConnectionInfo, FromMessageRequest, Message, MessageRequest, Payload},
    message::Message as MessageTrait,
};

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestMsg(u8);

#[test]
fn message_extractor_parses_and_advances() {
    let msg = TestMsg(42);
    let bytes = msg.to_bytes().unwrap();
    let mut payload = Payload {
        data: bytes.as_slice(),
    };
    let req = MessageRequest::default();

    let extracted = Message::<TestMsg>::from_message_request(&req, &mut payload).unwrap();
    assert_eq!(*extracted, msg);
    assert_eq!(payload.remaining(), 0);
}

#[test]
fn connection_info_reports_peer() {
    let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    let req = MessageRequest {
        peer_addr: Some(addr),
        app_data: HashMap::default(),
    };
    let mut payload = Payload::default();
    let info = ConnectionInfo::from_message_request(&req, &mut payload).unwrap();
    assert_eq!(info.peer_addr(), Some(addr));
}

#[test]
fn shared_state_extractor() {
    let mut data = HashMap::default();
    data.insert(
        std::any::TypeId::of::<u8>(),
        std::sync::Arc::new(42u8) as std::sync::Arc<dyn std::any::Any + Send + Sync>,
    );
    let req = MessageRequest {
        peer_addr: None,
        app_data: data,
    };
    let mut payload = Payload::default();

    let state =
        wireframe::extractor::SharedState::<u8>::from_message_request(&req, &mut payload).unwrap();
    assert_eq!(*state, 42);
}

#[test]
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
