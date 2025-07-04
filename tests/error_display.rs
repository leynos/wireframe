//! Tests for Display implementations on error types.

use wireframe::{push::PushError, response::WireframeError};

#[test]
fn push_error_messages() {
    assert_eq!(PushError::QueueFull.to_string(), "push queue full");
    assert_eq!(PushError::Closed.to_string(), "push queue closed");
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

    let proto = WireframeError::Protocol(ProtoErr);
    assert_eq!(proto.to_string(), "protocol error: ProtoErr");
}
