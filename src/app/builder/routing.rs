//! Routing and middleware configuration for `WireframeApp`.

use super::WireframeApp;
use crate::{
    app::{
        Packet,
        error::{Result, WireframeError},
        middleware_types::{Handler, Middleware},
    },
    codec::FrameCodec,
    serializer::Serializer,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Register a route that maps `id` to `handler`.
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::DuplicateRoute`] if a handler for `id`
    /// has already been registered.
    pub fn route(mut self, id: u32, handler: Handler<E>) -> Result<Self> {
        if self.handlers.contains_key(&id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.handlers.insert(id, handler);
        self.routes = tokio::sync::OnceCell::new();
        Ok(self)
    }

    /// Add a middleware component to the processing pipeline.
    ///
    /// # Errors
    ///
    /// This function currently always succeeds.
    pub fn wrap<M>(mut self, mw: M) -> Result<Self>
    where
        M: Middleware<E> + 'static,
    {
        self.middleware.push(Box::new(mw));
        self.routes = tokio::sync::OnceCell::new();
        Ok(self)
    }
}
