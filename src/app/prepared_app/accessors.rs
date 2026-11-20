//! Read-only accessors over a prepared application template.
//!
//! These are the parts of a [`PreparedApp`]'s surface that only report what
//! preparation installed: the protocol, the hooks derived from it, and the
//! message assembler. They live here so the module that builds and drives a
//! template stays small enough to read in one sitting.

use std::sync::Arc;

use super::PreparedApp;
use crate::{
    app::Packet,
    codec::FrameCodec,
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    serializer::Serializer,
};

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Get a clone of the configured protocol, if any.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// assert!(prepared.protocol().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn protocol(
        &self,
    ) -> Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>> {
        self.protocol.clone()
    }

    /// Return protocol hooks derived from the installed protocol.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let hooks = prepared.protocol_hooks();
    /// # let _ = hooks;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn protocol_hooks(&self) -> crate::hooks::ProtocolHooks<F::Frame, ()> {
        self.protocol.as_ref().map_or_else(
            crate::hooks::ProtocolHooks::default,
            crate::hooks::ProtocolHooks::from_protocol,
        )
    }

    /// Get the configured message assembler, if any.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// assert!(prepared.message_assembler().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn message_assembler(&self) -> Option<&Arc<dyn MessageAssembler>> {
        self.message_assembler.as_ref()
    }
}
