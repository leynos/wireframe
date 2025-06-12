use std::boxed::Box;

/// Configures routing and middleware for a `WireframeServer`.
///
/// The builder stores registered routes, services, and middleware
/// without enforcing an ordering. Methods return [`Result<Self>`] so
/// registrations can be chained ergonomically.
#[derive(Default)]
pub struct WireframeApp {
    routes: Vec<Route>,
    services: Vec<Service>,
    middleware: Vec<Box<dyn Middleware>>,
}

/// A route mapping a message identifier to a handler.
pub struct Route {
    /// Identifier of the incoming message this route handles.
    pub message_id: u32,
    /// Handler invoked when the message arrives.
    pub handler: Service,
}

/// Alias for boxed asynchronous handlers.
pub type Service = Box<dyn Fn() + Send + Sync>;

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
        if self.routes.iter().any(|r| r.message_id == id) {
            return Err(WireframeError::DuplicateRoute(id));
        }
        self.routes.push(Route {
            message_id: id,
            handler,
        });
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
}
