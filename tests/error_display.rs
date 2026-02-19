//! Tests for Display and Error trait implementations on error types.
//!
//! Verifies that error types provide human-readable messages via Display
//! and correctly expose underlying error sources via `Error::source`.
//! Implementing these traits keeps logs clear for operators,
//! and surfaces causal chains so developers can diagnose issues.
#![cfg(not(loom))]

use std::{error::Error, io};

use wireframe::{
    WireframeError,
    codec::{CodecError, FramingError},
    push::PushError,
};

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
    let proto = WireframeError::Protocol(ProtoErr);
    assert_eq!(proto.to_string(), "protocol error: ProtoErr");
    let proto_source = proto
        .source()
        .expect("protocol variant must expose its underlying source");
    assert_eq!(proto_source.to_string(), "boom");

    let duplicate_route = WireframeError::<ProtoErr>::DuplicateRoute(7);
    assert_eq!(
        duplicate_route.to_string(),
        "route id 7 was already registered"
    );
    assert!(
        duplicate_route.source().is_none(),
        "DuplicateRoute should not expose an error source"
    );
}

#[test]
fn wireframe_error_exposes_sources_for_io_and_codec() {
    let io = WireframeError::<ProtoErr>::from_io(io::Error::other("socket closed"));
    assert_eq!(io.to_string(), "transport error: socket closed");
    let source = io.source().expect("io variant must have source");
    let io_source = source
        .downcast_ref::<io::Error>()
        .expect("io source should be std::io::Error");
    assert_eq!(io_source.kind(), io::ErrorKind::Other);
    assert_eq!(io_source.to_string(), "socket closed");

    let wireframe_codec =
        WireframeError::<ProtoErr>::from_codec(CodecError::from(FramingError::EmptyFrame));
    let codec_source = wireframe_codec
        .source()
        .expect("Codec variant should expose an error source");
    let typed_codec_source = codec_source
        .downcast_ref::<CodecError>()
        .expect("codec source should be wireframe::codec::CodecError");
    assert!(
        matches!(
            typed_codec_source,
            CodecError::Framing(FramingError::EmptyFrame)
        ),
        "codec source should preserve the original framing error"
    );
}
