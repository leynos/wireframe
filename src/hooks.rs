//! Internal protocol hooks called by the connection actor.
//!
//! This module defines [`ProtocolHooks`], a container for optional callback
//! functions invoked during connection output. The hooks are placeholders for
//! the future `WireframeProtocol` trait described in the design documents.

/// Type alias for the `before_send` callback.
type BeforeSendHook<F> = Box<dyn FnMut(&mut F) + Send + 'static>;

/// Type alias for the `on_command_end` callback.
type OnCommandEndHook = Box<dyn FnMut() + Send + 'static>;

/// Callbacks used by the connection actor.
pub struct ProtocolHooks<F> {
    /// Invoked before a frame is written to the socket.
    pub before_send: Option<BeforeSendHook<F>>,
    /// Invoked once a command completes.
    pub on_command_end: Option<OnCommandEndHook>,
}

impl<F> Default for ProtocolHooks<F> {
    fn default() -> Self {
        Self {
            before_send: None,
            on_command_end: None,
        }
    }
}

impl<F> ProtocolHooks<F> {
    /// Run the `before_send` hook if registered.
    pub fn before_send(&mut self, frame: &mut F) {
        if let Some(hook) = &mut self.before_send {
            hook(frame);
        }
    }

    /// Run the `on_command_end` hook if registered.
    pub fn on_command_end(&mut self) {
        if let Some(hook) = &mut self.on_command_end {
            hook();
        }
    }
}
