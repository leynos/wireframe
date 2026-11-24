//! Deprecated compatibility wrapper for accepted connections.
//!
//! [`PreparedApp::handle_connection`] remains so callers that predate the
//! result-returning entry point keep compiling; it forwards to the shared
//! connection path and only logs a failure.

use tokio::io::{AsyncRead, AsyncWrite};

use super::PreparedApp;
use crate::{
    app::{Envelope, envelope::Packet},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Handle an accepted connection and log any processing failure.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// prepared.handle_connection(server).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle_connection<W>(&self, stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Err(error) = self.handle_connection_result(stream).await {
            log::warn!(
                "connection handling completed with error: correlation_id={:?}, error={error:?}",
                None::<u64>
            );
        }
    }
}
