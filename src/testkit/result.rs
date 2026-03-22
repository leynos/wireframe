//! Shared result and error types for `wireframe::testkit`.

use thiserror::Error;

use crate::{
    WireframeError,
    codec::CodecError,
    connection::ConnectionStateError,
    fragment::{FragmentationError, ReassemblyError},
    push::{PushConfigError, PushError},
};

/// Error type shared by `wireframe::testkit` helper APIs.
#[derive(Debug, Error)]
pub enum TestError {
    /// IO error surfaced while exercising test utilities.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Assertion or other message-driven failure.
    #[error("{0}")]
    Msg(String),
    /// Root crate error surfaced while exercising a helper.
    #[error("wireframe error: {0}")]
    Wireframe(WireframeError),
    /// Client-side error surfaced while exercising a helper.
    #[cfg(not(loom))]
    #[error(transparent)]
    Client(#[from] crate::client::ClientError),
    /// Server-side error surfaced while exercising a helper.
    #[cfg(not(loom))]
    #[error(transparent)]
    Server(#[from] crate::server::ServerError),
    /// Push-queue error surfaced while exercising a helper.
    #[error(transparent)]
    Push(#[from] PushError),
    /// Push configuration error surfaced while exercising a helper.
    #[error(transparent)]
    PushConfig(#[from] PushConfigError),
    /// Connection-state error surfaced while exercising a helper.
    #[error(transparent)]
    ConnectionState(#[from] ConnectionStateError),
    /// Fragment reassembly error surfaced while exercising a helper.
    #[error(transparent)]
    Reassembly(#[from] ReassemblyError),
    /// Fragmentation error surfaced while exercising a helper.
    #[error(transparent)]
    Fragmentation(#[from] FragmentationError),
    /// Codec error surfaced while exercising a helper.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// Serialization error surfaced while exercising a helper.
    #[error(transparent)]
    Encode(#[from] bincode::error::EncodeError),
    /// Deserialization error surfaced while exercising a helper.
    #[error(transparent)]
    Decode(#[from] bincode::error::DecodeError),
    /// Task join error surfaced while exercising a helper.
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    /// Timeout surfaced while exercising a helper.
    #[error(transparent)]
    Timeout(#[from] tokio::time::error::Elapsed),
    /// Oneshot receive error surfaced while exercising a helper.
    #[error(transparent)]
    OneshotRecv(#[from] tokio::sync::oneshot::error::RecvError),
    /// Oneshot try-receive error surfaced while exercising a helper.
    #[error(transparent)]
    OneshotTryRecv(#[from] tokio::sync::oneshot::error::TryRecvError),
    /// MPSC try-receive error surfaced while exercising a helper.
    #[error(transparent)]
    MpscTryRecv(#[from] tokio::sync::mpsc::error::TryRecvError),
    /// Integer parsing error surfaced while exercising a helper.
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),
    /// Integer conversion error surfaced while exercising a helper.
    #[error(transparent)]
    TryFromInt(#[from] std::num::TryFromIntError),
    /// UTF-8 decoding error surfaced while exercising a helper.
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    /// Owned UTF-8 decoding error surfaced while exercising a helper.
    #[error(transparent)]
    FromUtf8(#[from] std::string::FromUtf8Error),
    /// Address parsing error surfaced while exercising a helper.
    #[error(transparent)]
    AddrParse(#[from] std::net::AddrParseError),
}

impl From<String> for TestError {
    fn from(value: String) -> Self { Self::Msg(value) }
}

impl From<&str> for TestError {
    fn from(value: &str) -> Self { Self::Msg(value.to_string()) }
}

impl From<WireframeError> for TestError {
    fn from(value: WireframeError) -> Self { Self::Wireframe(value) }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for TestError {
    fn from(err: tokio::sync::mpsc::error::SendError<T>) -> Self { Self::Msg(err.to_string()) }
}

impl<T> From<tokio::sync::mpsc::error::TrySendError<T>> for TestError {
    fn from(err: tokio::sync::mpsc::error::TrySendError<T>) -> Self { Self::Msg(err.to_string()) }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for TestError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self { Self::Msg(err.to_string()) }
}

/// Shared result type for testkit helpers.
pub type TestResult<T = ()> = Result<T, TestError>;
