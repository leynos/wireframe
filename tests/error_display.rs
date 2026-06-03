//! Tests for Display and Error trait implementations on error types.
//!
//! Verifies that error types provide human-readable messages via Display
//! and correctly expose underlying error sources via `Error::source`.
//! Implementing these traits keeps logs clear for operators,
//! and surfaces causal chains so developers can diagnose issues.
#![cfg(not(loom))]

use std::{error::Error, io};

use wireframe::{
    NoProtocolError,
    WireframeError,
    client::ClientProtocolError,
    codec::{CodecError, FramingError},
    push::PushError,
    testkit::TestError,
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

#[derive(Debug)]
enum ExpectedSource {
    Io {
        message: &'static str,
        kind: io::ErrorKind,
    },
    Codec(FramingError),
}

#[derive(Debug, PartialEq, Eq)]
enum ActualSource<'a> {
    Io {
        message: String,
        kind: io::ErrorKind,
    },
    Codec {
        error: &'a FramingError,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum SourceExtractionError {
    UnexpectedType,
    UnexpectedCodecVariant,
}

fn extract_source<'a>(
    source: &'a (dyn Error + 'static),
) -> Result<ActualSource<'a>, SourceExtractionError> {
    if let Some(io_source) = source.downcast_ref::<io::Error>() {
        return Ok(ActualSource::Io {
            message: io_source.to_string(),
            kind: io_source.kind(),
        });
    }

    if let Some(codec_source) = source.downcast_ref::<CodecError>() {
        return match codec_source {
            CodecError::Framing(error) => Ok(ActualSource::Codec { error }),
            _ => Err(SourceExtractionError::UnexpectedCodecVariant),
        };
    }

    Err(SourceExtractionError::UnexpectedType)
}

fn source_matches(actual: &ActualSource<'_>, expected: &ExpectedSource) -> bool {
    match (actual, expected) {
        (
            ActualSource::Io {
                message: actual_message,
                kind: actual_kind,
            },
            ExpectedSource::Io {
                message: expected_message,
                kind: expected_kind,
            },
        ) => actual_kind == expected_kind && actual_message == expected_message,
        (
            ActualSource::Codec {
                error: actual_error,
            },
            ExpectedSource::Codec(expected_error),
        ) => *actual_error == expected_error,
        _ => false,
    }
}

fn assert_source_matches(source: &(dyn Error + 'static), expected: ExpectedSource) {
    let expected = std::convert::identity(expected);
    let actual = match extract_source(source) {
        Ok(actual) => actual,
        Err(error) => panic!("source should be a supported error source: {error:?}"),
    };

    assert!(
        source_matches(&actual, &expected),
        "source should match expected source\nactual: {actual:?}\nexpected: {expected:?}",
    );
}

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
fn wireframe_error_preserves_client_protocol_source_chain() {
    let protocol = ClientProtocolError::Deserialize(Box::new(io::Error::other("decode failed")));
    let wireframe = WireframeError::Protocol(protocol);
    let protocol_source = wireframe
        .source()
        .expect("protocol errors implementing Error should be exposed");

    assert!(
        protocol_source
            .downcast_ref::<ClientProtocolError>()
            .is_some(),
        "protocol source should retain its concrete client error type"
    );
    let decode_source = protocol_source
        .source()
        .expect("client protocol error should expose the decode source");
    assert_eq!(decode_source.to_string(), "decode failed");
}

#[test]
fn wireframe_error_unit_messages() {
    let io = WireframeError::<()>::from_io(io::Error::other("socket closed"));
    assert_eq!(io.to_string(), "transport error: socket closed");

    let codec = WireframeError::<()>::from_codec(CodecError::from(FramingError::EmptyFrame));
    assert_eq!(
        codec.to_string(),
        "codec error: framing error: empty frame not permitted"
    );

    let protocol = WireframeError::<()>::Protocol(());
    assert_eq!(protocol.to_string(), "protocol error: ()");

    let duplicate_route = WireframeError::<()>::DuplicateRoute(7);
    assert_eq!(
        duplicate_route.to_string(),
        "route id 7 was already registered"
    );
}

#[rstest::rstest]
#[case(
    WireframeError::<NoProtocolError>::from_io(io::Error::other("socket closed")),
    ExpectedSource::Io {
        message: "socket closed",
        kind: io::ErrorKind::Other,
    }
)]
#[case(
    WireframeError::<NoProtocolError>::from_codec(CodecError::from(FramingError::EmptyFrame)),
    ExpectedSource::Codec(FramingError::EmptyFrame)
)]
fn wireframe_error_default_exposes_sources(
    #[case] error: WireframeError<NoProtocolError>,
    #[case] expected: ExpectedSource,
) {
    let source = error
        .source()
        .expect("variant must expose its underlying source");
    assert_source_matches(source, expected);
}

#[test]
fn wireframe_error_default_duplicate_route_has_no_source() {
    let duplicate_route = WireframeError::<NoProtocolError>::DuplicateRoute(7);
    assert!(
        duplicate_route.source().is_none(),
        "DuplicateRoute should not expose an error source"
    );
}

#[test]
fn wireframe_error_display_allows_non_static_protocol_type() {
    struct BorrowedProtocolError<'a>(&'a str);

    impl std::fmt::Debug for BorrowedProtocolError<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "BorrowedProtocolError({:?})", self.0)
        }
    }

    let message = String::from("borrowed");
    let error = WireframeError::Protocol(BorrowedProtocolError(&message));

    assert_eq!(
        error.to_string(),
        "protocol error: BorrowedProtocolError(\"borrowed\")"
    );
}

#[test]
fn test_error_wireframe_from_preserves_display_prefix() {
    let error = TestError::from(WireframeError::DuplicateRoute(7));

    assert_eq!(
        error.to_string(),
        "wireframe error: route id 7 was already registered"
    );
}

#[rstest::rstest]
#[case(
    WireframeError::<ProtoErr>::from_io(io::Error::other("socket closed")),
    "transport error: socket closed",
    ExpectedSource::Io {
        message: "socket closed",
        kind: io::ErrorKind::Other,
    }
)]
#[case(
    WireframeError::<ProtoErr>::from_codec(CodecError::from(FramingError::EmptyFrame)),
    "codec error: framing error: empty frame not permitted",
    ExpectedSource::Codec(FramingError::EmptyFrame)
)]
fn wireframe_error_exposes_sources_for_io_and_codec(
    #[case] error: WireframeError<ProtoErr>,
    #[case] display: &str,
    #[case] expected: ExpectedSource,
) {
    assert_eq!(error.to_string(), display);
    let source = error.source().expect("variant must expose an error source");
    assert_source_matches(source, expected);
}
