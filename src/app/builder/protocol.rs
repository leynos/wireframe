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
    /// by the connection actor and the application response pipeline.
    ///
    /// A fresh [`ProtocolHooks`] value is built for each connection. The
    /// `before_send` hook runs for every `WireframeApp` handler response frame
    /// after serialization and codec wrapping, immediately before the frame is
    /// written to the transport. Because handler responses are driven through
    /// the per-connection [`crate::connection::ConnectionActor`], the
    /// `on_command_end`, stream-end terminator, and error hooks also run on the
    /// application path. Connection setup runs once per connection before the
    /// response actor starts, and the end-of-file hook runs when a connection's
    /// input ends mid-header or mid-frame. The direct response helpers on
    /// [`WireframeApp`] bypass `before_send` because they write to a
    /// caller-supplied stream.
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
        Self {
            protocol: Some(Arc::new(protocol)),
            ..self
        }
    }

    /// Install a [`MessageAssembler`] implementation.
    ///
    /// The assembler parses protocol-specific frame headers for multi-frame
    /// request assembly on the inbound pipeline. Wireframe applies the hook
    /// after any transport fragmentation reassembly and before handler
    /// dispatch, producing either buffered requests or streaming request
    /// bodies.
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
    /// let app: WireframeApp = WireframeApp::new()
    ///     .expect("builder")
    ///     .with_message_assembler(DemoAssembler);
    /// let _ = app;
    /// ```
    #[must_use]
    pub fn with_message_assembler(self, assembler: impl MessageAssembler + 'static) -> Self {
        Self {
            message_assembler: Some(Box::new(assembler)),
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
            .map_or_else(ProtocolHooks::default, ProtocolHooks::from_protocol)
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
    /// let app: WireframeApp = WireframeApp::new()
    ///     .expect("builder")
    ///     .with_message_assembler(DemoAssembler);
    /// assert!(app.message_assembler().is_some());
    /// ```
    ///
    /// The accessor borrows the application-owned assembler. It deliberately
    /// does not hand out a cloneable handle: the assembler is immutable and
    /// only called through `&self`, so a caller that needs to invoke it can do
    /// so through the returned reference. This is the
    /// [#598](https://github.com/leynos/wireframe/issues/598) disposition and
    /// replaces the former `Option<&Arc<dyn MessageAssembler>>` return type in
    /// the 0.4.0 release.
    #[must_use]
    pub fn message_assembler(&self) -> Option<&dyn MessageAssembler> {
        self.message_assembler.as_deref()
    }
}
