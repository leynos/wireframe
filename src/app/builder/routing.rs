//! Routing and middleware configuration for `WireframeApp`.

use super::WireframeApp;
use crate::{
    WireframeError,
    app::{
        Packet,
        Route,
        error::Result,
        middleware_types::{Handler, Middleware, ResponseHandler},
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
    pub fn route(self, id: u32, handler: Handler<E>) -> Result<Self> {
        self.insert_route(id, Route::Unit(handler))
    }

    /// Register a route whose handler returns the response to stream.
    ///
    /// This is the streaming counterpart to [`route`](Self::route). The
    /// handler receives the decoded request envelope and returns any
    /// [`Response`](crate::response::Response) variant; `Single`, `Vec`, and
    /// `Empty` complete immediately, while `Stream` and `MultiPacket` are
    /// driven by the connection actor.
    ///
    /// Every emitted frame is stamped with the request's correlation id and
    /// passes through fragmentation, outbound metrics, serialization, codec
    /// wrapping, and the protocol `before_send` hook. A streamed response is
    /// driven to completion before the next inbound frame on the connection is
    /// read. Middleware observes an empty response frame for these routes
    /// because the body travels in a deferred slot; see
    /// [`ServiceResponse`](crate::middleware::ServiceResponse).
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::DuplicateRoute`] if a handler for `id` has
    /// already been registered.
    pub fn route_response(self, id: u32, handler: ResponseHandler<E>) -> Result<Self> {
        self.insert_route(id, Route::Response(handler))
    }

    /// Insert a route after rejecting duplicate identifiers.
    fn insert_route(mut self, id: u32, route: Route<E>) -> Result<Self> {
        if self.handlers.contains_key(&id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.handlers.insert(id, route);
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
