//! Push-queue and session-registry accessors on a prepared application.

use std::sync::Arc;

use super::{Packet, PreparedApp, builder::push::with_codec_frame_limit};
use crate::{
    codec::FrameCodec,
    push::PushQueuesBuilder,
    serializer::Serializer,
    session::SessionRegistry,
};

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Return the registry that connections from this application register
    /// with.
    ///
    /// The registry is populated automatically as connections pass their
    /// preamble and is emptied again as they close.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let registry = prepared.session_registry();
    /// assert!(registry.active_ids().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn session_registry(&self) -> Arc<SessionRegistry<F::Frame>> {
        Arc::clone(&self.session_registry)
    }

    /// Return the push-queue template used for each new connection.
    ///
    /// A prepared application always yields a template: when the builder stored
    /// none, the default queue configuration is returned. The template is
    /// bounded by the application codec's frame limit, so a connection's push
    /// handle rejects a frame its codec could not encode instead of letting
    /// the encoder fail and end the connection.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let template = prepared.push_queues();
    /// # let _ = template;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn push_queues(&self) -> PushQueuesBuilder<F::Frame> {
        with_codec_frame_limit(self.push_queues.clone().unwrap_or_default(), &self.codec)
    }
}
