//! Trait for parsing frame metadata from a header without decoding the payload.

/// Parse frame metadata from a byte slice without decoding the payload.
///
/// # Examples
///
/// ```rust
/// use std::io;
/// struct Demo;
/// impl FrameMetadata for Demo {
///     type Frame = (u32, bytes::Bytes); // (id, payload)
///     type Error = io::Error;
///     fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
///         if src.len() < 4 {
///             return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "len"));
///         }
///         let id = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);
///         Ok(((id, bytes::Bytes::copy_from_slice(&src[4..])), src.len()))
///     }
/// }
/// ```
pub trait FrameMetadata {
    /// Fully deserialised frame envelope type. Implementations SHOULD keep
    /// payloads zero‑copy (e.g., using `bytes::Bytes`) to avoid allocations.
    type Frame;

    /// Error produced when parsing the metadata.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Parse frame metadata from `src`, returning the envelope and bytes
    /// consumed.
    ///
    /// # Errors
    /// Returns an error if the bytes cannot be interpreted as valid metadata.
    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error>;
}
