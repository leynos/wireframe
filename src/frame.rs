use async_trait::async_trait;

/// Trait defining how raw bytes are decoded into frames and how frames are
/// encoded back into bytes for transmission.
///
/// The `Frame` associated type represents a logical unit extracted from or
/// written to the wire. Errors are represented by the `Error` associated type,
/// which must implement [`std::error::Error`].
#[async_trait]
pub trait FrameProcessor: Send + Sync {
    /// Logical frame type extracted from the stream.
    type Frame;

    /// Error type returned by `decode` and `encode`.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Attempt to decode the next frame from `src`.
    async fn decode(&mut self, src: &[u8]) -> Result<Option<Self::Frame>, Self::Error>;

    /// Encode `frame` and append the bytes to `dst`.
    async fn encode(&mut self, frame: &Self::Frame, dst: &mut Vec<u8>) -> Result<(), Self::Error>;
}
