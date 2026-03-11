//! Shared helpers for codec-error regression tests.

use bytes::BytesMut;
use tokio_util::codec::Decoder;
use wireframe::{
    byte_order::write_network_u32,
    codec::{
        CodecError,
        CodecErrorContext,
        EofError,
        FrameCodec,
        LENGTH_HEADER_SIZE,
        LengthDelimitedFrameCodec,
        RecoveryPolicy,
        RecoveryPolicyHook,
    },
};
use wireframe_testing::{TestResult, encode_frame, new_test_codec};

pub(crate) struct StrictRecoveryHook;

impl RecoveryPolicyHook for StrictRecoveryHook {
    fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        RecoveryPolicy::Disconnect
    }
}

pub(crate) fn extract_eof_error(error: &std::io::Error) -> TestResult<EofError> {
    let mut current = error
        .get_ref()
        .map(|inner| inner as &(dyn std::error::Error + 'static));

    while let Some(err) = current {
        if let Some(eof) = err.downcast_ref::<EofError>() {
            return Ok(*eof);
        }
        current = err.source();
    }

    Err(format!("expected wrapped EofError, got {error}").into())
}

pub(crate) fn classify_eof(bytes: &[u8], consume_frame_before_eof: bool) -> TestResult<EofError> {
    let codec = LengthDelimitedFrameCodec::new(1024);
    let mut decoder = codec.decoder();
    let mut buffer = BytesMut::from(bytes);

    if consume_frame_before_eof {
        let frame = decoder.decode(&mut buffer)?;
        if frame.is_none() {
            return Err("expected a complete frame before EOF".into());
        }
    }

    match decoder.decode_eof(&mut buffer) {
        Ok(None) => Ok(EofError::CleanClose),
        Ok(Some(_)) => Err("unexpected extra frame while classifying EOF".into()),
        Err(error) => extract_eof_error(&error),
    }
}

pub(crate) fn encoded_default_frame(payload: &[u8]) -> TestResult<Vec<u8>> {
    let mut codec = new_test_codec(1024);
    encode_frame(&mut codec, payload.to_vec()).map_err(Into::into)
}

pub(crate) fn truncated_payload_wire(payload: &[u8]) -> TestResult<Vec<u8>> {
    let expected = u32::try_from(payload.len())?;
    let partial_len = payload.len().saturating_sub(1);
    let mut wire = Vec::with_capacity(LENGTH_HEADER_SIZE + partial_len);
    wire.extend_from_slice(&write_network_u32(expected));
    let partial = payload
        .get(..partial_len)
        .ok_or("partial payload slice should be available")?;
    wire.extend_from_slice(partial);
    Ok(wire)
}
