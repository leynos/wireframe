#![cfg(not(loom))]
//! Tests for extracting shared state from message requests.
//!
//! They verify successful extraction and error handling when state is missing.

use rstest::{fixture, rstest};
use wireframe::extractor::{
    ExtractError,
    FromMessageRequest,
    MessageRequest,
    Payload,
    SharedState,
};

#[expect(
    clippy::allow_attributes,
    reason = "rstest single-line fixtures need allow to avoid unfulfilled lint expectations"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "rstest occasionally misses the expected lint for single-line fixtures on stable"
)]
#[expect(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
fn request() -> MessageRequest { MessageRequest::default() }

#[expect(
    clippy::allow_attributes,
    reason = "rstest single-line fixtures need allow to avoid unfulfilled lint expectations"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "rstest occasionally misses the expected lint for single-line fixtures on stable"
)]
#[expect(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
#[fixture]
fn empty_payload() -> Payload<'static> { Payload::default() }

#[rstest]
fn shared_state_extractor_returns_data(
    mut request: MessageRequest,
    mut empty_payload: Payload<'static>,
) {
    request.insert_state(5u32);
    let extracted = SharedState::<u32>::from_message_request(&request, &mut empty_payload)
        .expect("failed to extract shared state");
    assert_eq!(*extracted, 5);
}

#[rstest]
fn missing_shared_state_returns_error(
    request: MessageRequest,
    mut empty_payload: Payload<'static>,
) {
    let err = SharedState::<u32>::from_message_request(&request, &mut empty_payload)
        .err()
        .expect("missing state error expected");
    assert!(matches!(err, ExtractError::MissingState(_)));
}
