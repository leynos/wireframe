//! BDD world for codec-error regression scenarios backed by
//! `wireframe_testing`.

use bytes::BytesMut;
use rstest::fixture;
use tokio_util::codec::Decoder;
use wireframe::{
    byte_order::write_network_u32,
    codec::{
        CodecError,
        CodecErrorContext,
        EofError,
        FrameCodec,
        FramingError,
        LengthDelimitedFrameCodec,
        RecoveryPolicy,
        RecoveryPolicyHook,
    },
};
pub use wireframe_testing::TestResult;
use wireframe_testing::{ObservabilityHandle, encode_frame, new_test_codec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PartialEofKind {
    MidHeader,
    MidFrame,
}

struct StrictRecoveryHook;

impl RecoveryPolicyHook for StrictRecoveryHook {
    fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        RecoveryPolicy::Disconnect
    }
}

#[derive(Default)]
pub struct CodecErrorRegressionsWorld {
    obs: Option<ObservabilityHandle>,
    last_eof: Option<EofError>,
    partial_eofs: Vec<EofError>,
}

impl std::fmt::Debug for CodecErrorRegressionsWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodecErrorRegressionsWorld")
            .field("obs", &self.obs.as_ref().map(|_| ".."))
            .field("last_eof", &self.last_eof)
            .field("partial_eofs", &self.partial_eofs)
            .finish()
    }
}

#[rustfmt::skip]
#[fixture]
pub fn codec_error_regressions_world() -> CodecErrorRegressionsWorld {
    CodecErrorRegressionsWorld::default()
}

impl CodecErrorRegressionsWorld {
    pub fn acquire(&mut self) {
        let mut obs = ObservabilityHandle::new();
        obs.clear();
        self.obs = Some(obs);
        self.last_eof = None;
        self.partial_eofs.clear();
    }

    pub fn record_default_oversized_error(&mut self) -> TestResult {
        let error = CodecError::Framing(FramingError::OversizedFrame {
            size: 2048,
            max: 1024,
        });
        self.record_metric(&error, error.default_recovery_policy())
    }

    pub fn observe_clean_close(&mut self) -> TestResult {
        let wire = encoded_default_frame(&[1, 2, 3, 4])?;
        let actual = classify_eof(&wire, true)?;
        self.last_eof = Some(actual);
        let error = CodecError::Eof(actual);
        self.record_metric(&error, error.default_recovery_policy())
    }

    pub fn observe_partial_header(&mut self) -> TestResult {
        let wire = partial_header_wire()?;
        let actual = classify_eof(&wire, false)?;
        self.partial_eofs.push(actual);
        Ok(())
    }

    pub fn observe_partial_payload(&mut self) -> TestResult {
        let wire = partial_payload_wire(&[1, 2, 3, 4])?;
        let actual = classify_eof(&wire, false)?;
        self.partial_eofs.push(actual);
        Ok(())
    }

    pub fn record_strict_hook_override(&mut self) -> TestResult {
        let hook = StrictRecoveryHook;
        let error = CodecError::Framing(FramingError::OversizedFrame {
            size: 2048,
            max: 1024,
        });
        let policy = hook.recovery_policy(&error, &CodecErrorContext::new());
        self.record_metric(&error, policy)
    }

    pub fn assert_codec_error_count(
        &mut self,
        error_type: &str,
        recovery_policy: &str,
        expected: u64,
    ) -> TestResult {
        let obs = self
            .obs
            .as_mut()
            .ok_or("observability handle not acquired")?;
        obs.snapshot();
        obs.assert_codec_error_counter(error_type, recovery_policy, expected)
            .map_err(Into::into)
    }

    pub fn assert_last_clean_close(&self) -> TestResult {
        match self.last_eof {
            Some(EofError::CleanClose) => Ok(()),
            Some(other) => Err(format!("expected clean close, got {other:?}").into()),
            None => Err("no EOF classification recorded".into()),
        }
    }

    fn assert_partial_kind(&self, index: usize, expected: PartialEofKind) -> TestResult {
        let actual = self
            .partial_eofs
            .get(index)
            .copied()
            .ok_or_else(|| format!("missing partial EOF classification at index {index}"))?;

        match (expected, actual) {
            (PartialEofKind::MidHeader, EofError::MidHeader { .. })
            | (PartialEofKind::MidFrame, EofError::MidFrame { .. }) => Ok(()),
            _ => Err(format!("expected {expected:?}, got {actual:?}").into()),
        }
    }

    fn record_metric(&mut self, error: &CodecError, policy: RecoveryPolicy) -> TestResult {
        let obs = self
            .obs
            .as_ref()
            .ok_or("observability handle not acquired")?;
        metrics::with_local_recorder(obs.recorder(), || {
            wireframe::metrics::inc_codec_error(error.error_type(), policy.as_str());
        });
        Ok(())
    }

    pub fn assert_first_partial_mid_header(&self) -> TestResult {
        self.assert_partial_kind(0, PartialEofKind::MidHeader)
    }

    pub fn assert_second_partial_mid_frame(&self) -> TestResult {
        self.assert_partial_kind(1, PartialEofKind::MidFrame)
    }
}

fn encoded_default_frame(payload: &[u8]) -> TestResult<Vec<u8>> {
    let mut codec = new_test_codec(1024);
    encode_frame(&mut codec, payload.to_vec()).map_err(Into::into)
}

fn partial_header_wire() -> TestResult<Vec<u8>> {
    let wire = encoded_default_frame(&[1, 2, 3, 4])?;
    wire.get(..2)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "expected at least two header bytes".into())
}

fn partial_payload_wire(payload: &[u8]) -> TestResult<Vec<u8>> {
    let expected = u32::try_from(payload.len())?;
    let partial_len = payload.len().saturating_sub(1);
    let mut wire = Vec::with_capacity(4 + partial_len);
    wire.extend_from_slice(&write_network_u32(expected));
    wire.extend_from_slice(
        payload
            .get(..partial_len)
            .ok_or("partial payload slice should be available")?,
    );
    Ok(wire)
}

fn classify_eof(bytes: &[u8], consume_frame_before_eof: bool) -> TestResult<EofError> {
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

fn extract_eof_error(error: &std::io::Error) -> TestResult<EofError> {
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

#[cfg(test)]
mod tests {
    //! Unit tests for the BDD world helper logic.

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(PartialEofKind::MidHeader, partial_header_wire)]
    #[case(PartialEofKind::MidFrame, || partial_payload_wire(&[1, 2, 3, 4]))]
    fn classify_partial_inputs(
        #[case] expected: PartialEofKind,
        #[case] build: fn() -> TestResult<Vec<u8>>,
    ) -> TestResult {
        let actual = classify_eof(&build()?, false)?;
        match (expected, actual) {
            (PartialEofKind::MidHeader, EofError::MidHeader { .. })
            | (PartialEofKind::MidFrame, EofError::MidFrame { .. }) => Ok(()),
            _ => Err(format!("expected {expected:?}, got {actual:?}").into()),
        }
    }

    #[test]
    fn classify_clean_close_after_consuming_frame() -> TestResult {
        let actual = classify_eof(&encoded_default_frame(&[1, 2, 3, 4])?, true)?;
        if actual != EofError::CleanClose {
            return Err(format!("expected clean close, got {actual:?}").into());
        }
        Ok(())
    }

    #[test]
    fn strict_hook_overrides_default_drop_policy() {
        let hook = StrictRecoveryHook;
        let error = CodecError::Framing(FramingError::OversizedFrame {
            size: 2048,
            max: 1024,
        });
        let policy = hook.recovery_policy(&error, &CodecErrorContext::new());
        assert_eq!(policy, RecoveryPolicy::Disconnect);
    }
}
