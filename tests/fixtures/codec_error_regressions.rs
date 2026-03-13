//! BDD world for codec-error regression scenarios backed by
//! `wireframe_testing`.

use rstest::fixture;
use wireframe::codec::{
    CodecError,
    CodecErrorContext,
    EofError,
    FramingError,
    RecoveryPolicy,
    RecoveryPolicyHook,
};
use wireframe_testing::ObservabilityHandle;
pub(crate) use wireframe_testing::TestResult;

#[path = "../common/codec_error_regression_support.rs"]
mod codec_error_regression_support;

use codec_error_regression_support::{
    StrictRecoveryHook,
    classify_eof,
    encoded_default_frame,
    truncated_payload_wire,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PartialEofKind {
    MidHeader,
    MidFrame,
}

#[derive(Default)]
pub(crate) struct CodecErrorRegressionsWorld {
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
pub(crate) fn codec_error_regressions_world() -> CodecErrorRegressionsWorld {
    CodecErrorRegressionsWorld::default()
}

impl CodecErrorRegressionsWorld {
    pub(crate) fn acquire(&mut self) {
        let mut obs = ObservabilityHandle::new();
        obs.clear();
        self.obs = Some(obs);
        self.last_eof = None;
        self.partial_eofs.clear();
    }

    pub(crate) fn record_default_oversized_error(&mut self) -> TestResult {
        let error = CodecError::Framing(FramingError::OversizedFrame {
            size: 2048,
            max: 1024,
        });
        self.record_metric(&error, error.default_recovery_policy())
    }

    pub(crate) fn observe_clean_close(&mut self) -> TestResult {
        let wire = encoded_default_frame(&[1, 2, 3, 4])?;
        let actual = classify_eof(&wire, true)?;
        self.last_eof = Some(actual);
        let error = CodecError::Eof(actual);
        self.record_metric(&error, error.default_recovery_policy())
    }

    pub(crate) fn observe_partial_header(&mut self) -> TestResult {
        let wire = partial_header_wire()?;
        let actual = classify_eof(&wire, false)?;
        self.partial_eofs.push(actual);
        Ok(())
    }

    pub(crate) fn observe_partial_payload(&mut self) -> TestResult {
        let wire = partial_payload_wire(&[1, 2, 3, 4])?;
        let actual = classify_eof(&wire, false)?;
        self.partial_eofs.push(actual);
        Ok(())
    }

    pub(crate) fn record_strict_hook_override(&mut self) -> TestResult {
        let hook = StrictRecoveryHook;
        let error = CodecError::Framing(FramingError::OversizedFrame {
            size: 2048,
            max: 1024,
        });
        let policy = hook.recovery_policy(&error, &CodecErrorContext::new());
        self.record_metric(&error, policy)
    }

    pub(crate) fn assert_codec_error_count(
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

    pub(crate) fn assert_last_clean_close(&self) -> TestResult {
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

        if matches_partial_eof_kind(expected, &actual) {
            Ok(())
        } else {
            Err(format!("expected {expected:?}, got {actual:?}").into())
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

    pub(crate) fn assert_first_partial_mid_header(&self) -> TestResult {
        self.assert_partial_kind(0, PartialEofKind::MidHeader)
    }

    pub(crate) fn assert_second_partial_mid_frame(&self) -> TestResult {
        self.assert_partial_kind(1, PartialEofKind::MidFrame)
    }
}

fn partial_header_wire() -> TestResult<Vec<u8>> {
    let wire = encoded_default_frame(&[1, 2, 3, 4])?;
    wire.get(..2)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "expected at least two header bytes".into())
}

fn partial_payload_wire(payload: &[u8]) -> TestResult<Vec<u8>> { truncated_payload_wire(payload) }

fn matches_partial_eof_kind(expected: PartialEofKind, actual: &EofError) -> bool {
    matches!(
        (expected, actual),
        (PartialEofKind::MidHeader, EofError::MidHeader { .. })
            | (PartialEofKind::MidFrame, EofError::MidFrame { .. })
    )
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
        if matches_partial_eof_kind(expected, &actual) {
            Ok(())
        } else {
            Err(format!("expected {expected:?}, got {actual:?}").into())
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
