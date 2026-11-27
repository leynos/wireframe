//! Compatibility wrapper for accepted connections.
//!
//! [`PreparedApp::handle_connection`] is the infallible entry point: it
//! forwards to the shared connection path and logs a failure rather than
//! returning it. The deprecated member is
//! [`WireframeApp::handle_connection`](crate::app::WireframeApp::handle_connection),
//! whose callers this method lets keep compiling; the replacement itself is not
//! deprecated.

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
            // The wrapper keeps its log-and-return contract; a structured
            // field lets a subscriber filter these without parsing the message.
            tracing::warn!(
                error = %error,
                "connection handling completed with error"
            );
        }
    }
}
