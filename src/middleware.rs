//! Traits and helpers for request middleware.
//!
//! Middleware components implement [`Transform`] to wrap services and
//! process `ServiceRequest` instances before passing them along the chain.

use async_trait::async_trait;

/// Incoming request wrapper passed through middleware.
#[derive(Debug)]
pub struct ServiceRequest;

/// Response produced by a handler or middleware.
#[derive(Debug, Default)]
pub struct ServiceResponse;

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
    /// Create a new [`Next`] wrapping the given service.
    #[inline]
    #[must_use]
    /// Creates a new `Next` instance wrapping a reference to the given service.
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
    /// #         Ok(ServiceResponse)
    /// #     }
    /// # }
    /// let service = MyService;
    /// let next = Next::new(&service);
    /// ```
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
/// #         Ok(ServiceResponse)
/// #     }
/// # }
/// let mw = from_fn(logging);
/// ```
pub fn from_fn<F>(f: F) -> FromFn<F> { FromFn::new(f) }

pub struct FnService<S, F> {
    service: S,
    f: F,
}

#[async_trait]
impl<S, F, Fut> Service for FnService<S, F>
where
    S: Service + 'static,
    F: for<'a> Fn(ServiceRequest, Next<'a, S>) -> Fut + Send + Sync + Clone,
    Fut: std::future::Future<Output = Result<ServiceResponse, S::Error>> + Send,
{
    type Error = S::Error;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let next = Next::new(&self.service);
        (self.f.clone())(req, next).await
    }
}

#[async_trait]
impl<S, F, Fut> Transform<S> for FromFn<F>
where
    S: Service + 'static,
    F: for<'a> Fn(ServiceRequest, Next<'a, S>) -> Fut + Send + Sync + Clone,
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
