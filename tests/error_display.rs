//! Tests for Display and Error trait implementations on error types.
//!
//! Verifies that error types provide human-readable messages via Display
//! and correctly expose underlying error sources via `Error::source`.
//! Implementing these traits keeps logs clear for operators,
//! and surfaces causal chains so developers can diagnose issues.
#![cfg(not(loom))]

use std::error::Error;

use wireframe::{WireframeError, push::PushError};

#[rstest::rstest]
#[case(PushError::QueueFull, "push queue full")]
#[case(PushError::Closed, "push queue closed")]
fn push_error_messages(#[case] err: PushError, #[case] expected: &str) {
    assert_eq!(err.to_string(), expected);
}

#[derive(Debug)]
struct ProtoErr;

impl std::fmt::Display for ProtoErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("boom") }
}

impl std::error::Error for ProtoErr {}

#[test]
fn wireframe_error_messages() {
    let io_error = std::io::Error::other("socket closed");
    let io = WireframeError::<ProtoErr>::Io(io_error);
    assert_eq!(io.to_string(), "transport error: socket closed");

    let source = io.source().expect("io variant must have source");
    assert_eq!(source.to_string(), "socket closed");

    let proto = WireframeError::Protocol(ProtoErr);
    assert_eq!(proto.to_string(), "protocol error: ProtoErr");
    assert!(
        proto.source().is_none(),
        "protocol variant does not expose a source for generic payloads"
    );

    let duplicate_route = WireframeError::<()>::DuplicateRoute(7);
    assert_eq!(
        duplicate_route.to_string(),
        "route id 7 was already registered"
    );
    assert!(
        duplicate_route.source().is_none(),
        "DuplicateRoute should not expose an error source"
    );
}
