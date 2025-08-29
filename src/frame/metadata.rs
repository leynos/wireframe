//! Trait for parsing frame metadata from a header without decoding the payload.

/// Parse frame metadata from a byte slice.
pub trait FrameMetadata {
    /// Fully deserialised frame type.
    type Frame;

    /// Error produced when parsing the metadata.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Parse frame metadata from `src`, returning the frame and bytes consumed.
    ///
    /// # Errors
    /// Returns an error if the bytes cannot be interpreted as valid metadata.
    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error>;
}
