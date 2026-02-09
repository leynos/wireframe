//! Protocol and message assembly configuration for `WireframeApp`.

use std::sync::Arc;

use super::WireframeApp;
use crate::{
    app::Packet,
    codec::FrameCodec,
    hooks::{ProtocolHooks, WireframeProtocol},
    message_assembler::MessageAssembler,
    serializer::Serializer,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Install a [`WireframeProtocol`] implementation.
    ///
    /// The protocol defines hooks for connection setup, frame modification, and
    /// command completion. It is wrapped in an [`Arc`] and stored for later use
    /// by the connection actor.
    ///
    /// At present, the protocol must use `ProtocolError = ()`. This keeps the
    /// protocol object safe for dynamic dispatch, maintains a uniform
    /// interface across connections, and avoids leaking application-specific
    /// error types into the builder API.
    #[must_use]
    pub fn with_protocol<P>(self, protocol: P) -> Self
    where
        P: WireframeProtocol<Frame = F::Frame, ProtocolError = ()> + 'static,
    {
        WireframeApp {
            protocol: Some(Arc::new(protocol)),
            ..self
        }
    }

    /// Install a [`MessageAssembler`] implementation.
    ///
    /// The assembler parses protocol-specific frame headers to support
    /// multi-frame request assembly once the inbound pipeline integrates it.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::{
    ///     app::WireframeApp,
    ///     message_assembler::{MessageAssembler, ParsedFrameHeader},
    /// };
    ///
    /// struct DemoAssembler;
    ///
    /// impl MessageAssembler for DemoAssembler {
    ///     fn parse_frame_header(&self, _payload: &[u8]) -> Result<ParsedFrameHeader, std::io::Error> {
    ///         Err(std::io::Error::new(
    ///             std::io::ErrorKind::InvalidData,
    ///             "unimplemented",
    ///         ))
    ///     }
    /// }
    ///
    /// let app = WireframeApp::new()
    ///     .expect("builder")
    ///     .with_message_assembler(DemoAssembler);
    /// let _ = app;
    /// ```
    #[must_use]
    pub fn with_message_assembler(self, assembler: impl MessageAssembler + 'static) -> Self {
        WireframeApp {
            message_assembler: Some(Arc::new(assembler)),
            ..self
        }
    }

    /// Get a clone of the configured protocol, if any.
    ///
    /// Returns `None` if no protocol was installed via [`with_protocol`](Self::with_protocol).
    #[must_use]
    pub fn protocol(
        &self,
    ) -> Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>> {
        self.protocol.clone()
    }

    /// Return protocol hooks derived from the installed protocol.
    ///
    /// If no protocol is installed, returns default (no-op) hooks.
    #[must_use]
    pub fn protocol_hooks(&self) -> ProtocolHooks<F::Frame, ()> {
        self.protocol
            .as_ref()
            .map(ProtocolHooks::from_protocol)
            .unwrap_or_default()
    }

    /// Get the configured message assembler, if any.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::{
    ///     app::WireframeApp,
    ///     message_assembler::{MessageAssembler, ParsedFrameHeader},
    /// };
    ///
    /// struct DemoAssembler;
    ///
    /// impl MessageAssembler for DemoAssembler {
    ///     fn parse_frame_header(&self, _payload: &[u8]) -> Result<ParsedFrameHeader, std::io::Error> {
    ///         Err(std::io::Error::new(
    ///             std::io::ErrorKind::InvalidData,
    ///             "unimplemented",
    ///         ))
    ///     }
    /// }
    ///
    /// let app = WireframeApp::new()
    ///     .expect("builder")
    ///     .with_message_assembler(DemoAssembler);
    /// assert!(app.message_assembler().is_some());
    /// ```
    #[must_use]
    pub fn message_assembler(&self) -> Option<&Arc<dyn MessageAssembler>> {
        self.message_assembler.as_ref()
    }
}
