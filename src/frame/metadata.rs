//! Trait for parsing frame metadata from a header without decoding the payload.

/// Parse frame metadata from a byte slice without decoding the payload.
///
/// # Examples
///
/// ```rust,no_run
/// use std::io;
///
/// use bytes::Bytes;
/// use wireframe::frame::FrameMetadata;
///
/// struct Demo;
/// impl FrameMetadata for Demo {
///     type Frame = (u32, Bytes); // (id, payload)
///     type Error = io::Error;
///     fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
///         if src.len() < 4 {
///             return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "len"));
///         }
///         let id = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);
///         Ok(((id, Bytes::copy_from_slice(&src[4..])), src.len()))
///     }
/// }
/// ```
pub trait FrameMetadata {
    /// Fully deserialised frame envelope type.
    /// Prefer avoiding unnecessary allocations. Zero‑copy is only possible
    /// when the caller supplies an owned, ref‑counted buffer (e.g., `Bytes`).
    /// With the current `&[u8]` input, copying the payload is typically required.
    type Frame;

    /// Error produced when parsing the metadata.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Parse frame metadata from `src`, returning the envelope and the number
    /// of bytes consumed.
    ///
    /// # Errors
    /// Returns an error if the bytes cannot be interpreted as valid metadata.
    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error>;
}
