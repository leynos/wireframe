//! End-to-end regressions for rejecting malformed envelopes before dispatch.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rstest::rstest;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::{LengthDelimitedFrameCodec, MAX_FRAME_LENGTH},
    message::Message,
    serializer::BincodeSerializer,
};
use wireframe_testing::{TestResult, drive_with_codec_payloads};

use super::codec_error_regression_support::oversized_envelope_bytes;

/// Inputs that distinguish valid dispatch from malformed-frame rejection.
#[derive(Clone, Copy, Debug)]
enum EnvelopeInput {
    Valid,
    TrailingBytes,
    OversizedContainer,
}

#[rstest]
#[case::valid(EnvelopeInput::Valid, 1)]
#[case::trailing_bytes(EnvelopeInput::TrailingBytes, 0)]
#[case::oversized_container(EnvelopeInput::OversizedContainer, 0)]
#[tokio::test]
async fn malformed_envelopes_never_reach_the_route(
    #[case] input: EnvelopeInput,
    #[case] expected_calls: usize,
) -> TestResult {
    let called = Arc::new(AtomicUsize::new(0));
    let counter = called.clone();
    let codec = LengthDelimitedFrameCodec::new(1024);
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_codec(codec.clone())
        .route(
            1,
            Arc::new(move |_: &Envelope| {
                let counter = counter.clone();
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                })
            }),
        )?;
    let bytes = match input {
        EnvelopeInput::Valid => Envelope::new(1, None, vec![42]).to_bytes()?,
        EnvelopeInput::TrailingBytes => {
            let mut bytes = Envelope::new(1, None, vec![42]).to_bytes()?;
            bytes.push(0);
            bytes
        }
        EnvelopeInput::OversizedContainer => {
            oversized_envelope_bytes(u64::try_from(MAX_FRAME_LENGTH)? + 1)?
        }
    };
    let responses = drive_with_codec_payloads(app, &codec, vec![bytes]).await?;
    assert_eq!(called.load(Ordering::SeqCst), expected_calls);
    assert_eq!(responses.len(), expected_calls);
    Ok(())
}
