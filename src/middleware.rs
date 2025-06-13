use async_trait::async_trait;

#[derive(Debug)]
pub struct ServiceRequest;

#[derive(Debug, Default)]
pub struct ServiceResponse;

pub struct Next<'a, S: Service + ?Sized> {
    service: &'a S,
}

impl<'a, S: Service + ?Sized> Next<'a, S> {
    pub fn new(service: &'a S) -> Self {
        Self { service }
    }

    /// Call the next service with the provided request.
    ///
    /// # Errors
    ///
    /// Propagates any error returned by the wrapped service.
    pub async fn call(&mut self, req: ServiceRequest) -> Result<ServiceResponse, S::Error> {
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
