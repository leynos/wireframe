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
    ///
    /// # Examples
    ///
    /// ```
    /// let service = MyService::default();
    /// let next = Next::new(&service);
    /// ```
    pub const fn new(service: &'a S) -> Self {
        Self { service }
    }

    /// Call the next service with the given request.
    ///
    /// # Errors
    ///
    /// Asynchronously invokes the next service in the middleware chain with the given request.
    ///
    /// Returns the response from the wrapped service, or propagates any error produced.
    ///
    /// # Examples
    ///
    /// ```
    /// # use your_crate::{ServiceRequest, ServiceResponse, Next, Service};
    /// # struct DummyService;
    /// # #[async_trait::async_trait]
    /// # impl Service for DummyService {
    /// #     type Error = std::convert::Infallible;
    /// #     async fn call(&self, _req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
    /// #         Ok(ServiceResponse::default())
    /// #     }
    /// # }
    /// # let service = DummyService;
    /// let next = Next::new(&service);
    /// let req = ServiceRequest {};
    /// let res = tokio_test::block_on(next.call(req));
    /// assert!(res.is_ok());
    /// ```
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
    type Service: Service;

    /// Create a new middleware service wrapping `service`.
    async fn transform(&self, service: S) -> Self::Service;
}
