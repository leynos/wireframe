//! Request frames shared by the protocol tests.
//!
//! Prepared-connection tests feed a byte buffer, not an envelope, so each one
//! has to serialize and length-prefix a request first. Keeping that encoding in
//! one place stops the two tests that drive prepared connections from drifting
//! apart. Include this module with
//! `#[path = "common/protocol_request.rs"] mod protocol_request;`.

use wireframe::{
    app::Envelope,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestResult, encode_frame};

/// Encode one request envelope into a length-delimited frame.
pub fn request_frame(id: u32) -> TestResult<Vec<u8>> {
    let payload = BincodeSerializer.serialize(&Envelope::new(id, Some(7), Vec::new()))?;
    let mut codec = tokio_util::codec::LengthDelimitedCodec::builder()
        .max_frame_length(1024)
        .new_codec();
    Ok(encode_frame(&mut codec, payload)?)
}
