//! Reusable fixtures for prepared-application integration tests.

use std::sync::Arc;

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestResult, decode_frames, encode_frame};

/// Concrete application type shared by prepared-application integration tests.
pub type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Build a handler that accepts an envelope without changing it.
pub fn handler() -> Handler<Envelope> { Arc::new(|_: &Envelope| Box::pin(async {})) }

/// Encode an envelope into a frame for the default test transport.
pub fn build_frame(id: u32, payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let envelope = Envelope::new(id, Some(7), payload);
    let encoded = serializer.serialize(&envelope)?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, encoded)?)
}

/// Decode a single response frame and return its envelope payload.
pub fn response_payload(bytes: &[u8]) -> TestResult<Vec<u8>> {
    let frames = decode_frames(bytes)?;
    let [frame] = frames.as_slice() else {
        return Err("expected one response frame".into());
    };
    let serializer = BincodeSerializer;
    let (response, _) = serializer.deserialize::<Envelope>(frame)?;
    Ok(wireframe::app::Packet::into_parts(response).into_payload())
}
