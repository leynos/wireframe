//! Canonical error and result types for the crate.
//!
//! This module defines the single public `WireframeError` surface used by
//! application setup and runtime frame processing.

/// Top-level error type exposed by `wireframe`.
///
/// `WireframeError` distinguishes setup-time route conflicts from runtime
/// transport, protocol, and codec failures.
#[derive(Debug)]
pub enum WireframeError<E = ()> {
    /// A route with the provided identifier was already registered.
    DuplicateRoute(u32),
    /// An error in the underlying transport (for example, a socket close).
    Io(std::io::Error),
    /// A protocol-defined logical error.
    Protocol(E),
    /// A codec-layer error with structured context.
    Codec(crate::codec::CodecError),
}

impl<E> From<E> for WireframeError<E> {
    fn from(error: E) -> Self { Self::Protocol(error) }
}

impl<E> WireframeError<E> {
    /// Convert an I/O error into a `WireframeError`.
    #[must_use]
    pub fn from_io(error: std::io::Error) -> Self { Self::Io(error) }

    /// Convert a codec error into a `WireframeError`.
    #[must_use]
    pub fn from_codec(error: crate::codec::CodecError) -> Self { Self::Codec(error) }

    /// Returns true if this error represents a clean connection close.
    #[must_use]
    pub fn is_clean_close(&self) -> bool {
        matches!(self, Self::Codec(codec_error) if codec_error.is_clean_close())
    }
}

impl<E: std::fmt::Debug> std::fmt::Display for WireframeError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateRoute(id) => write!(f, "route id {id} was already registered"),
            Self::Io(error) => write!(f, "transport error: {error}"),
            Self::Protocol(error) => write!(f, "protocol error: {error:?}"),
            Self::Codec(error) => write!(f, "codec error: {error}"),
        }
    }
}

impl<E> std::error::Error for WireframeError<E>
where
    E: std::fmt::Debug + std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::DuplicateRoute(_) => None,
        }
    }
}

/// Canonical result alias used by `wireframe` public APIs.
pub type Result<T> = std::result::Result<T, WireframeError<()>>;
