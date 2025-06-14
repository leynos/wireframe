use std::boxed::Box;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Configures routing and middleware for a `WireframeServer`.
///
/// The builder stores registered routes, services, and middleware
/// without enforcing an ordering. Methods return [`Result<Self>`] so
/// registrations can be chained ergonomically.
#[derive(Default)]
pub struct WireframeApp {
    routes: HashMap<u32, Service>,
    services: Vec<Service>,
    middleware: Vec<Box<dyn Middleware>>,
}

/// Alias for boxed asynchronous handlers.
///
/// A `Service` is a boxed function returning a [`Future`], enabling
/// asynchronous execution of message handlers.
pub type Service = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Trait representing middleware components.
pub trait Middleware: Send + Sync {}

/// Top-level error type for application setup.
#[derive(Debug)]
pub enum WireframeError {
    /// A route with the provided identifier was already registered.
    DuplicateRoute(u32),
}

/// Result type used throughout the builder API.
pub type Result<T> = std::result::Result<T, WireframeError>;

impl WireframeApp {
    /// Construct a new empty application builder.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses the
    /// [`Result`] type for forward compatibility.
    pub fn new() -> Result<Self> {
        Ok(Self::default())
    }

    /// Register a route that maps `id` to `handler`.
    ///
    /// # Errors
    ///
    /// Returns [`WireframeError::DuplicateRoute`] if a handler for `id`
    /// has already been registered.
    pub fn route(mut self, id: u32, handler: Service) -> Result<Self> {
        if self.routes.contains_key(&id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.routes.insert(id, handler);
        Ok(self)
    }

    /// Register a handler discovered by attribute macros or other means.
    ///
    /// # Errors
    ///
    /// This function always succeeds currently but uses [`Result`] for
    /// consistency with other builder methods.
    pub fn service(mut self, handler: Service) -> Result<Self> {
        self.services.push(handler);
        Ok(self)
    }

    /// Add a middleware component to the processing pipeline.
    ///
    /// # Errors
    ///
    /// This function currently always succeeds.
    pub fn wrap<M>(mut self, mw: M) -> Result<Self>
    where
        M: Middleware + 'static,
    {
        self.middleware.push(Box::new(mw));
        Ok(self)
    }

    /// Handle an accepted connection.
    ///
    /// This placeholder simply drops the stream. Future implementations
    /// will decode frames and dispatch them to registered handlers.
    pub async fn handle_connection<S>(&self, _stream: S)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        // Connection handling will be implemented later.
        tokio::task::yield_now().await;
    }
}
