use async_trait::async_trait;

#[derive(Debug)]
pub struct ServiceRequest;

#[derive(Debug, Default)]
pub struct ServiceResponse;

pub struct Next<'a, S: Service + ?Sized> {
    service: &'a S,
}

impl<'a, S: Service + ?Sized> Next<'a, S> {
    /// Creates a new `Next` instance that wraps a reference to the given service.
    ///
    /// # Examples
    ///
    /// ```
    /// let service = MyService::default();
    /// let next = Next::new(&service);
    /// ```
    pub fn new(service: &'a S) -> Self {
        Self { service }
    }

    /// Call the next service with the provided request.
    ///
    /// # Errors
    ///
    /// Asynchronously calls the wrapped service with the given request, returning its response or error.
    ///
    /// # Examples
    ///
    /// ```
    /// # use your_crate::{Next, ServiceRequest, ServiceResponse, Service};
    /// # struct DummyService;
    /// # #[async_trait::async_trait]
    /// # impl Service for DummyService {
    /// #     type Error = std::convert::Infallible;
    /// #     async fn call(&self, _req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
    /// #         Ok(ServiceResponse)
    /// #     }
    /// # }
    /// # let service = DummyService;
    /// let next = Next::new(&service);
    /// # tokio_test::block_on(async {
    /// let response = next.call(ServiceRequest).await.unwrap();
    /// # });
    /// ```
    pub async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, S::Error> {
        self.service.call(req).await
    }
}

#[async_trait]
pub trait Service: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn call(&self, req: ServiceRequest) -> Result<ServiceResponse, Self::Error>;
}

#[async_trait]
pub trait Transform<S>: Send + Sync
where
    S: Service,
{
    type Output: Service;

    async fn transform(&self, service: S) -> Self::Output;
}
