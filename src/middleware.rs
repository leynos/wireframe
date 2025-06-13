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
    /// Creates a new `Next` instance wrapping a reference to the given service.
    #[must_use]
    /// Creates a new `Next` instance wrapping a reference to the given service.
    ///
    /// # Examples
    ///
    /// ```
    /// # use your_crate::{Next, Service, ServiceRequest};
    /// # struct MyService;
    /// # impl Service for MyService {
    /// #     type Error = std::convert::Infallible;
    /// #     async fn call(&self, _req: ServiceRequest) -> Result<super::ServiceResponse, Self::Error> {
    /// #         Ok(super::ServiceResponse)
    /// #     }
    /// # }
    /// let service = MyService;
    /// let next = Next::new(&service);
    /// ```
    pub fn new(service: &'a S) -> Self {
        Self { service }
    }

    /// Call the next service with the given request.
    ///
    /// # Errors
    ///
    /// Asynchronously invokes the wrapped service with the given request.
    ///
    /// Returns a response produced by the service, or an error if the service fails to handle the request.
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
    /// Wrapped service produced by the middleware.
    type Output: Service;

    /// Create a new middleware service wrapping `service`.
    async fn transform(&self, service: S) -> Self::Output;
}
