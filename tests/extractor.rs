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
