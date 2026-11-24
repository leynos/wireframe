//! Request middleware building blocks.
//!
//! Middleware can inspect or modify [`ServiceRequest`] values before they reach
//! the underlying [`Service`]. Implement [`Transform`] to wrap services or use
//! [`from_fn`] to create middleware from an async function.

mod handler_service;
mod service_response;
#[cfg(all(test, not(loom)))]
mod service_response_tests;

use async_trait::async_trait;

pub use self::{handler_service::HandlerService, service_response::ServiceResponse};

/// Generic container for request and response frame data.
#[derive(Debug, Default)]
pub struct FrameContainer<F> {
    /// Frame value transformed by middleware or consumed by a handler.
    frame: F,
}

impl<F> FrameContainer<F> {
    /// Create a new container holding `frame` bytes.
    #[must_use]
    pub const fn new(frame: F) -> Self { Self { frame } }

    /// Borrow the inner frame data.
    #[must_use]
    pub const fn frame(&self) -> &F { &self.frame }

    /// Mutable access to the frame data.
    #[must_use]
    pub const fn frame_mut(&mut self) -> &mut F { &mut self.frame }

    /// Consume the container, returning the frame.
    #[must_use]
    pub fn into_inner(self) -> F { self.frame }
}

/// Incoming request wrapper passed through middleware.
#[derive(Debug)]
pub struct ServiceRequest {
    /// Mutable payload presented to the middleware chain.
    inner: FrameContainer<Vec<u8>>,
    /// Correlation metadata preserved across middleware transformations.
    correlation_id: Option<u64>,
}

impl ServiceRequest {
    /// Create a new [`ServiceRequest`] from raw frame bytes.
    #[must_use]
    pub const fn new(frame: Vec<u8>, correlation_id: Option<u64>) -> Self {
        Self {
            inner: FrameContainer::new(frame),
            correlation_id,
        }
    }

    /// Borrow the underlying frame bytes.
    #[must_use]
    pub const fn frame(&self) -> &[u8] { self.inner.frame().as_slice() }

    /// Return the correlation identifier associated with this request, if any.
    #[must_use]
    pub const fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    /// Set or clear the correlation identifier on the request.
    #[must_use]
    pub const fn set_correlation_id(&mut self, correlation_id: Option<u64>) -> &mut Self {
        self.correlation_id = correlation_id;
        self
    }

    /// Mutable access to the inner frame bytes.
    #[must_use]
    pub const fn frame_mut(&mut self) -> &mut Vec<u8> { self.inner.frame_mut() }

    /// Consume the request, returning the inner frame bytes.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> { self.inner.into_inner() }
}

/// Continuation used by middleware to call the next service in the chain.
pub struct Next<'a, S>
where
    S: Service + ?Sized,
{
    /// Borrowed next service; borrowing prevents continuation after its scope.
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
    pub const fn new(service: &'a S) -> Self { Self { service } }

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
///
/// Middleware that participates in a response route must mutate the incoming
/// [`ServiceResponse`] and return it. A response installed by
/// `WireframeApp::route_response` travels in a deferred slot on that same
/// value; constructing a fresh [`ServiceResponse`] discards the slot, so the
/// handler's `Response` is dropped and the route emits one frame carrying the
/// fresh response's bytes instead.
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
    #[must_use = "use the returned middleware service"]
    async fn transform(&self, service: S) -> Self::Output;
}

/// Middleware created from an asynchronous function.
///
/// The function receives a [`ServiceRequest`] and a [`Next`] reference to invoke
/// the remaining middleware chain. It must return a [`ServiceResponse`] wrapped
/// in a [`Result`]. The error type is the same as the wrapped service.
pub struct FromFn<F> {
    /// User callback invoked once for each request.
    f: F,
}

impl<F> FromFn<F> {
    /// Construct middleware from the provided asynchronous function.
    pub const fn new(f: F) -> Self { Self { f } }
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
pub const fn from_fn<F>(f: F) -> FromFn<F> { FromFn::new(f) }

/// Service wrapper that applies a middleware function to requests.
///
/// Created by [`FromFn::transform`], this type owns the wrapped service and the
/// middleware function value. The function is invoked on each request with a
/// [`ServiceRequest`] and [`Next`] continuation.
pub struct FnService<S, F> {
    /// Wrapped service invoked by the continuation.
    service: S,
    /// Callback invoked for each request.
    f: F,
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
        (self.f)(req, next).await
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
            f: self.f.clone(),
        }
    }
}
