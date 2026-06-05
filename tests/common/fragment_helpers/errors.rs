//! Error types shared by fragment transport integration helpers.

use thiserror::Error;
use tokio::sync::mpsc;
use wireframe::fragment::ReassemblyError;

pub type TestResult<T = ()> = Result<T, TestError>;

/// Error type for fragment transport tests.
#[derive(Debug, Error)]
pub enum TestError {
    /// Test setup failed.
    #[error("test setup failed: {0}")]
    Setup(&'static str),
    /// Fragmentation operation failed.
    #[error("fragmentation failed: {0}")]
    Fragmentation(#[from] wireframe::fragment::FragmentationError),
    /// Encoding operation failed.
    #[error("encoding failed: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    /// Decoding operation failed.
    #[error("decoding failed: {0}")]
    Decode(#[from] bincode::error::DecodeError),
    /// Reassembly operation failed.
    #[error("reassembly failed: {0}")]
    Reassembly(#[from] ReassemblyError),
    /// Send operation failed.
    #[error("send failed: {0}")]
    Send(String),
    /// Application error.
    #[error("application error: {0}")]
    App(wireframe::WireframeError),
    /// Other error.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
    /// Test assertion failed.
    #[error("assertion failed: {0}")]
    Assertion(String),
    /// IO operation failed.
    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),
    /// Operation timed out.
    #[error("timeout: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    /// Task join failed.
    #[error("task join failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl<T> From<mpsc::error::SendError<T>> for TestError {
    fn from(err: mpsc::error::SendError<T>) -> Self { TestError::Send(err.to_string()) }
}

impl From<wireframe::WireframeError> for TestError {
    fn from(err: wireframe::WireframeError) -> Self { TestError::App(err) }
}
