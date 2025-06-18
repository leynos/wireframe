use std::{any::TypeId, collections::HashMap, sync::Arc};

use wireframe::extractor::{
    ExtractError,
    FromMessageRequest,
    MessageRequest,
    Payload,
    SharedState,
};

#[test]
fn shared_state_extractor_returns_data() {
    let state: SharedState<u32> = 5u32.into();
    let mut map = HashMap::new();
    map.insert(
        TypeId::of::<SharedState<u32>>(),
        Arc::new(state.clone()) as Arc<dyn std::any::Any + Send + Sync>,
    );
    let req = MessageRequest {
        peer_addr: None,
        app_data: map,
    };
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
