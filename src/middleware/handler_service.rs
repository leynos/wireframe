//! Middleware-aware service that dispatches one registered route.
//!
//! [`HandlerService`] is the prepared form of a route: the handler plus its
//! transformed middleware chain. It lives in its own module so the middleware
//! building blocks in the parent module stay under the module size limit, and
//! so the two ways a route becomes a service — borrowing an unprepared
//! builder's route and consuming a prepared one — are read side by side.

use std::{convert::Infallible, sync::Arc};

use async_trait::async_trait;

use super::{Service, ServiceRequest, ServiceResponse};
use crate::app::{Handler, Packet, PacketParts, ResponseHandler, Route};

/// Service that invokes a stored route handler and middleware chain.
pub struct HandlerService<E: Packet> {
    /// Route identifier retained for diagnostics and dispatch metadata.
    id: u32,
    /// Type-erased middleware and handler service.
    svc: Box<dyn Service<Error = Infallible> + Send + Sync>,
    /// Marker to bind the generic parameter `E` without storing a value.
    _marker: std::marker::PhantomData<E>,
}

impl<E: Packet> HandlerService<E> {
    /// Create a new [`HandlerService`] for the given route `id` and `handler`.
    #[must_use]
    pub fn new(id: u32, handler: Handler<E>) -> Self {
        Self {
            id,
            svc: Box::new(RouteService {
                id,
                handler,
                _marker: std::marker::PhantomData,
            }),
            _marker: std::marker::PhantomData,
        }
    }

    /// Construct a `HandlerService` from an existing service implementation.
    #[must_use]
    pub fn from_service(id: u32, svc: impl Service<Error = Infallible> + 'static) -> Self {
        Self {
            id,
            svc: Box::new(svc),
            _marker: std::marker::PhantomData,
        }
    }

    /// Construct a `HandlerService` for whichever route kind is registered.
    ///
    /// Used by the compatibility path, which still owns an unprepared builder
    /// and therefore clones each route's handler `Arc`.
    #[must_use]
    pub(crate) fn from_route(id: u32, route: &Route<E>) -> Self {
        match route {
            Route::Unit(handler) => Self::new(id, Arc::clone(handler)),
            Route::Response(handler) => Self::from_service(
                id,
                ResponseRouteService {
                    id,
                    handler: Arc::clone(handler),
                    _marker: std::marker::PhantomData,
                },
            ),
        }
    }

    /// Construct a `HandlerService` by consuming a registered route.
    ///
    /// Preparation owns the builder, so the handler `Arc` moves into the
    /// service instead of being cloned. The public `Handler` and
    /// `ResponseHandler` aliases stay `Arc`-shaped because a caller may
    /// legitimately retain a clone of a handler it supplied.
    #[must_use]
    pub(crate) fn from_owned_route(id: u32, route: Route<E>) -> Self {
        match route {
            Route::Unit(handler) => Self::new(id, handler),
            Route::Response(handler) => Self::from_service(
                id,
                ResponseRouteService {
                    id,
                    handler,
                    _marker: std::marker::PhantomData,
                },
            ),
        }
    }

    /// Returns the route identifier associated with this service.
    #[must_use]
    pub const fn id(&self) -> u32 { self.id }
}

/// Concrete service that invokes a packet handler for one route.
struct RouteService<E: Packet> {
    /// Route identifier copied into generated responses.
    id: u32,
    /// Handler invoked after middleware accepts the request.
    handler: Handler<E>,
    /// Keeps the packet type associated with the erased service.
    _marker: std::marker::PhantomData<E>,
}

#[async_trait]
impl<E: Packet> Service for RouteService<E> {
    type Error = Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        // The handler only borrows the envelope, allowing us to consume it
        // afterwards to extract the response payload.
        let env = E::from_parts(PacketParts::new(
            self.id,
            req.correlation_id(),
            req.into_inner(),
        ));
        (self.handler.as_ref())(&env).await;
        let parts = env.into_parts();
        let correlation_id = parts.correlation_id();
        let payload = parts.into_payload();
        Ok(ServiceResponse::new(payload, correlation_id))
    }
}

/// Concrete service that invokes a response-building handler for one route.
struct ResponseRouteService<E: Packet> {
    /// Route identifier copied into the request envelope.
    id: u32,
    /// Handler invoked after middleware accepts the request.
    handler: ResponseHandler<E>,
    /// Keeps the packet type associated with the erased service.
    _marker: std::marker::PhantomData<E>,
}

#[async_trait]
impl<E: Packet> Service for ResponseRouteService<E> {
    type Error = Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let correlation_id = req.correlation_id();
        let env = E::from_parts(PacketParts::new(self.id, correlation_id, req.into_inner()));
        let response = (self.handler.as_ref())(&env).await;
        let deferred = Arc::new(std::sync::Mutex::new(Some(response)));
        Ok(ServiceResponse::with_deferred(
            Vec::new(),
            correlation_id,
            deferred,
        ))
    }
}

#[async_trait]
impl<E: Packet> Service for HandlerService<E> {
    type Error = Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        self.svc.call(req).await
    }
}
