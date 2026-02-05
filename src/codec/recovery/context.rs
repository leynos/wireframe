//! Structured context for codec error recovery and logging.

use std::net::SocketAddr;

/// Structured context for codec error logging and diagnostics.
///
/// This struct captures connection-specific information to include in
/// structured logs and metrics when codec errors occur.
///
/// # Examples
///
/// ```
/// use wireframe::codec::CodecErrorContext;
///
/// let ctx = CodecErrorContext::new()
///     .with_connection_id(42)
///     .with_correlation_id(123);
///
/// assert_eq!(ctx.connection_id, Some(42));
/// assert_eq!(ctx.correlation_id, Some(123));
/// ```
#[derive(Clone, Debug, Default)]
pub struct CodecErrorContext {
    /// Unique identifier for the connection.
    pub connection_id: Option<u64>,

    /// Remote peer address.
    pub peer_address: Option<SocketAddr>,

    /// Correlation identifier from the frame, if available.
    ///
    /// This helps attribute errors to specific requests when debugging.
    pub correlation_id: Option<u64>,

    /// Codec instance state for debugging.
    ///
    /// May include sequence numbers, bytes processed, or other codec-specific
    /// state information.
    pub codec_state: Option<String>,
}

impl CodecErrorContext {
    /// Create a new empty context.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Set the connection identifier.
    #[must_use]
    pub fn with_connection_id(mut self, id: u64) -> Self {
        self.connection_id = Some(id);
        self
    }

    /// Set the peer address.
    #[must_use]
    pub fn with_peer_address(mut self, addr: SocketAddr) -> Self {
        self.peer_address = Some(addr);
        self
    }

    /// Set the correlation identifier.
    #[must_use]
    pub fn with_correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set codec-specific state information.
    #[must_use]
    pub fn with_codec_state(mut self, state: impl Into<String>) -> Self {
        self.codec_state = Some(state.into());
        self
    }
}
