//! Routing and middleware configuration for `WireframeApp`.

use super::WireframeApp;
use crate::{
    WireframeError,
    app::{
        Packet,
        error::Result,
        middleware_types::{Handler, Middleware, StreamingHandler},
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
    /// has already been registered, buffered or streaming.
    pub fn route(mut self, id: u32, handler: Handler<E>) -> Result<Self> {
        if self.handlers.contains_key(&id) || self.streaming_handlers.contains_key(&id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.handlers.insert(id, handler);
        Ok(self)
    }

    /// Register a streaming route that maps `id` to `handler`.
    ///
    /// A streaming handler receives [`RequestParts`](crate::request::RequestParts)
    /// and a [`RequestBodyStream`](crate::request::RequestBodyStream) instead
    /// of a fully assembled envelope. When the first frame for `id` completes
    /// the message, the handler still receives a one-chunk stream. Middleware
    /// registered with [`wrap`](Self::wrap) does not run for streaming routes.
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::DuplicateRoute`] if a handler for `id` has
    /// already been registered, buffered or streaming.
    pub fn streaming_route(mut self, id: u32, handler: StreamingHandler) -> Result<Self> {
        if self.handlers.contains_key(&id) || self.streaming_handlers.contains_key(&id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.streaming_handlers.insert(id, handler);
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
        Ok(self)
    }
}
