//! Request middleware building blocks.
//!
//! Middleware can inspect or modify [`ServiceRequest`] values before they reach
//! the underlying [`Service`]. Implement [`Transform`] to wrap services or use
//! [`from_fn`] to create middleware from an async function.

use std::{convert::Infallible, sync::Arc};

use async_trait::async_trait;

/// Generic container for request and response frame data.
#[derive(Debug, Default)]
pub struct FrameContainer<F> {
    frame: F,
}

impl<F> FrameContainer<F> {
    /// Create a new container holding `frame` bytes.
    #[must_use]
    pub fn new(frame: F) -> Self { Self { frame } }

    /// Borrow the inner frame data.
    #[must_use]
    pub fn frame(&self) -> &F { &self.frame }

    /// Mutable access to the frame data.
    #[must_use]
    pub fn frame_mut(&mut self) -> &mut F { &mut self.frame }

    /// Consume the container, returning the frame.
    #[must_use]
    pub fn into_inner(self) -> F { self.frame }
}

/// Incoming request wrapper passed through middleware.
#[derive(Debug)]
pub struct ServiceRequest {
    inner: FrameContainer<Vec<u8>>,
}

impl ServiceRequest {
    /// Create a new [`ServiceRequest`] from raw frame bytes.
    #[must_use]
    pub fn new(frame: Vec<u8>) -> Self {
        Self {
            inner: FrameContainer::new(frame),
        }
    }

    /// Borrow the underlying frame bytes.
    #[must_use]
    pub fn frame(&self) -> &[u8] { self.inner.frame().as_slice() }

    /// Mutable access to the inner frame bytes.
    #[must_use]
    pub fn frame_mut(&mut self) -> &mut Vec<u8> { self.inner.frame_mut() }

    /// Consume the request, returning the inner frame bytes.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> { self.inner.into_inner() }
}

/// Response produced by a handler or middleware.
#[derive(Debug, Default)]
pub struct ServiceResponse {
    inner: FrameContainer<Vec<u8>>,
}

impl ServiceResponse {
    /// Create a new [`ServiceResponse`] containing the given frame bytes.
    #[must_use]
    pub fn new(frame: Vec<u8>) -> Self {
        Self {
            inner: FrameContainer::new(frame),
        }
    }

    /// Borrow the inner frame bytes.
    #[must_use]
    pub fn frame(&self) -> &[u8] { self.inner.frame().as_slice() }

    /// Mutable access to the response frame bytes.
    #[must_use]
    pub fn frame_mut(&mut self) -> &mut Vec<u8> { self.inner.frame_mut() }

    /// Consume the response, yielding the raw frame bytes.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> { self.inner.into_inner() }
}

/// Continuation used by middleware to call the next service in the chain.
pub struct Next<'a, S>
where
    S: Service + ?Sized,
{
    service: &'a S,
}

impl<'a, S> Next<'a, S>
where
    S: Service + ?Sized,
{
    /// Creates a new `Next` instance wrapping a reference to `service`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// # use async_trait::async_trait;
    /// # use wireframe::middleware::{Next, Service, ServiceRequest, ServiceResponse};
    /// # struct MyService;
    /// # #[async_trait]
    /// # impl Service for MyService {
    /// #     type Error = std::convert::Infallible;
    /// #     async fn call(&self, _req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
    /// #         Ok(ServiceResponse::default())
    /// #     }
    /// # }
    /// let service = MyService;
    /// let next = Next::new(&service);
    /// ```
    #[inline]
    #[must_use]
    pub fn new(service: &'a S) -> Self { Self { service } }

    /// Call the next service with the provided request.
    ///
    /// # Errors
    ///
    /// Asynchronously invokes the wrapped service with the given request.
    ///
    /// Returns a response produced by the service, or an error if the service fails to handle the
    /// request.
    #[must_use = "await the returned future"]
    pub async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, S::Error> {
        self.service.call(req).await
    }
}

/// Trait representing an asynchronous service.
#[async_trait]
pub trait Service: Send + Sync {
    /// Error type returned by the service.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Handle the incoming request and produce a response.
    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error>;
}

/// Factory for wrapping services with middleware.
#[async_trait]
pub trait Transform<S>: Send + Sync
where
    S: Service,
{
    /// Middleware-wrapped service produced by `transform`.
    type Output: Service;

    /// Create a new middleware service wrapping `service`.
    #[inline]
    #[allow(clippy::inline_fn_without_body, unused_attributes)]
    #[must_use = "use the returned middleware service"]
    async fn transform(&self, service: S) -> Self::Output;
}

/// Middleware created from an asynchronous function.
///
/// The function receives a [`ServiceRequest`] and a [`Next`] reference to invoke
/// the remaining middleware chain. It must return a [`ServiceResponse`] wrapped
/// in a [`Result`]. The error type is the same as the wrapped service.
pub struct FromFn<F> {
    f: F,
}

impl<F> FromFn<F> {
    /// Construct middleware from the provided asynchronous function.
    pub fn new(f: F) -> Self { Self { f } }
}

/// Convenience constructor to build middleware from an async function.
///
/// # Examples
///
/// ```
/// use wireframe::middleware::{from_fn, ServiceRequest, ServiceResponse, Next};
///
/// async fn logging(req: ServiceRequest, next: Next<'_, MyService>)
///     -> Result<ServiceResponse, std::convert::Infallible>
/// {
///     println!("request: {:?}", req);
///     let res = next.call(req).await?;
///     println!("response: {:?}", res);
///     Ok(res)
/// }
///
/// # struct MyService;
/// # #[async_trait::async_trait]
/// # impl wireframe::middleware::Service for MyService {
/// #     type Error = std::convert::Infallible;
/// #     async fn call(&self, _req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
/// #         Ok(ServiceResponse::default())
/// #     }
/// # }
/// let mw = from_fn(logging);
/// ```
pub fn from_fn<F>(f: F) -> FromFn<F> { FromFn::new(f) }

/// Service wrapper that applies a middleware function to requests.
///
/// Created by [`FromFn::transform`], this type owns the wrapped service and an
/// `Arc` to the middleware function. The function is invoked on each request
/// with a [`ServiceRequest`] and [`Next`] continuation.
pub struct FnService<S, F> {
    service: S,
    f: Arc<F>,
}

#[async_trait]
impl<S, F, Fut> Service for FnService<S, F>
where
    S: Service + 'static,
    F: for<'a> Fn(ServiceRequest, Next<'a, S>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ServiceResponse, S::Error>> + Send,
{
    type Error = S::Error;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let next = Next::new(&self.service);
        (self.f.as_ref())(req, next).await
    }
}

#[async_trait]
impl<S, F, Fut> Transform<S> for FromFn<F>
where
    S: Service + 'static,
    F: for<'a> Fn(ServiceRequest, Next<'a, S>) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<ServiceResponse, S::Error>> + Send,
{
    type Output = FnService<S, F>;

    async fn transform(&self, service: S) -> Self::Output {
        FnService {
            service,
            f: Arc::new(self.f.clone()),
        }
    }
}

use crate::app::{Handler, Packet};

/// Service that invokes a stored route handler and middleware chain.
pub struct HandlerService<E: Packet> {
    id: u32,
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

    /// Returns the route identifier associated with this service.
    #[must_use]
    pub const fn id(&self) -> u32 { self.id }
}

struct RouteService<E: Packet> {
    id: u32,
    handler: Handler<E>,
    _marker: std::marker::PhantomData<E>,
}

#[async_trait]
impl<E: Packet> Service for RouteService<E> {
    type Error = Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        // The handler only borrows the envelope, allowing us to consume it
        // afterwards to extract the response payload.
        let env = E::from_parts(self.id, req.into_inner());
        (self.handler.as_ref())(&env).await;
        let (_, bytes) = env.into_parts();
        Ok(ServiceResponse::new(bytes))
    }
}

#[async_trait]
impl<E: Packet> Service for HandlerService<E> {
    type Error = Infallible;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        self.svc.call(req).await
    }
}
