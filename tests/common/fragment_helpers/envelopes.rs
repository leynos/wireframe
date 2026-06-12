//! Fragment envelope construction helpers for integration tests.

use wireframe::{
    app::{Envelope, Packet},
    fragment::{FragmentationConfig, Fragmenter, encode_fragment_payload},
};

use super::{TestError, TestResult};

/// Fragment an envelope into multiple fragment envelopes.
///
/// Returns the original envelope wrapped in a vec if the payload fits in a
/// single fragment, otherwise returns the fragmented envelopes.
///
/// # Errors
///
/// Returns an error if fragmentation or fragment payload encoding fails.
pub fn fragment_envelope(env: &Envelope, fragmenter: &Fragmenter) -> TestResult<Vec<Envelope>> {
    let parts = env.clone().into_parts();
    let id = parts.id();
    let correlation = parts.correlation_id();
    let payload = parts.into_payload();

    if payload.len() <= fragmenter.max_fragment_size().get() {
        return Ok(vec![Envelope::new(id, correlation, payload)]);
    }

    let envelopes = fragmenter
        .fragment_bytes(payload)?
        .into_iter()
        .map(|fragment| {
            let (header, payload) = fragment.into_parts();
            encode_fragment_payload(header, &payload)
                .map(|encoded| Envelope::new(id, correlation, encoded))
                .map_err(TestError::from)
        })
        .collect::<Result<Vec<_>, TestError>>()?;

    Ok(envelopes)
}

/// Build envelopes from a request, optionally fragmenting.
///
/// # Errors
///
/// Returns an error if fragmentation fails when `should_fragment` is true.
pub fn build_envelopes(
    request: Envelope,
    config: &FragmentationConfig,
    should_fragment: bool,
) -> TestResult<Vec<Envelope>> {
    if should_fragment {
        let fragmenter = Fragmenter::new(config.fragment_payload_cap);
        fragment_envelope(&request, &fragmenter)
    } else {
        Ok(vec![request])
    }
}
