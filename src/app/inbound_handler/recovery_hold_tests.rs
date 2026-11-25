//! Tests for the end-of-stream exit of a read made while a step is pending.
//!
//! [`hold_or_end`] is the one place a `decode_eof` failure ends the connection
//! instead of being buffered. The error it returns is what the connection
//! reports to teardown and what `report_truncation` inspects, so it must be the
//! decoder's own error rather than a rebuilt description: a typed payload such
//! as `CodecError::Framing` carries the recovery classification, and a source
//! chain is what makes a transport failure diagnosable.

use std::time::Duration;

use super::{test_support::*, *};
use crate::{
    app::recovery_codec::{DecodeFailure, DecodedItem},
    codec::{CodecError, FramingError},
};

/// Error a decoder reports for a frame whose declared length is too large.
fn oversized_error() -> io::Error {
    CodecError::Framing(FramingError::OversizedFrame {
        size: 129,
        max: 128,
    })
    .into()
}

/// Read the typed codec error a returned error carries.
fn codec_error_in_returned(error: &io::Error) -> &CodecError {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<CodecError>())
        .expect("the returned error must still carry its typed codec payload")
}

/// A `decode_eof` failure ends the connection and keeps its typed payload.
///
/// The previous implementation reported the failure and then rebuilt the error
/// it returned. A rebuilt error keeps the `io::ErrorKind` but loses the typed
/// `CodecError` payload and the source chain, so a large-prefix failure at EOF
/// would reach teardown as an opaque I/O error.
#[test]
fn a_decode_eof_failure_returns_the_original_typed_error() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let failure = DecodeFailure::new(oversized_error(), DecodeOrigin::DecodeEof, 4);
    let item = DecodedItem::<u8>::Failure(failure);

    let error = match hold_or_end(item, &coordinator) {
        Ok(_) => panic!("a decode_eof failure must end the connection"),
        Err(error) => error,
    };

    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "the original kind must survive"
    );
    assert!(
        matches!(
            codec_error_in_returned(&error),
            CodecError::Framing(FramingError::OversizedFrame {
                size: 129,
                max: 128
            })
        ),
        "the typed framing payload must survive, not be rebuilt from display text"
    );
    assert_eq!(
        hook.applied_policies(),
        vec![RecoveryPolicy::Disconnect],
        "the failure must be reported once, as the disconnect it is"
    );
    assert_eq!(
        hook.consultations(),
        0,
        "a connection that is already ending must not consult the hook"
    );
}

/// A non-EOF failure is held for the next dispatch and never reported here.
///
/// Only an end-of-stream failure is terminal while another step is pending. An
/// ordinary decode failure still has a frame after it, so its policy belongs to
/// the next read, and reporting it early would apply the wrong one.
#[test]
fn a_decode_failure_is_held_for_the_next_dispatch() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());
    let item = DecodedItem::<u8>::Failure(protocol_failure());

    let held = hold_or_end(item, &coordinator).expect("a decode failure is not terminal");

    assert!(
        matches!(held, DecodedItem::Failure(_)),
        "the failure must be passed on unchanged"
    );
    assert!(
        hook.applied_policies().is_empty(),
        "nothing may be reported before the policy is applied"
    );
}

/// A decoded frame passes through untouched.
#[test]
fn a_frame_is_passed_through() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let coordinator = RecoveryCoordinator::new(Some(&hook), tight_config());

    let held =
        hold_or_end(DecodedItem::Frame(7_u8), &coordinator).expect("a frame is not a failure");

    assert!(
        matches!(held, DecodedItem::Frame(7)),
        "a decoded frame must reach the dispatch step unchanged"
    );
    assert!(hook.applied_policies().is_empty());
}
