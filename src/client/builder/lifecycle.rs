//! Lifecycle hook methods for `WireframeClientBuilder`.

use std::{future::Future, sync::Arc};

use super::WireframeClientBuilder;
use crate::{
    client::{ClientError, hooks::LifecycleHooks},
    serializer::Serializer,
};

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    /// Register a callback invoked when the connection is established.
    ///
    /// The callback can perform authentication or other setup tasks and returns
    /// connection-specific state stored for the connection's lifetime. This
    /// hook is invoked after the preamble exchange (if configured) succeeds.
    ///
    /// # Type Parameters
    ///
    /// This method changes the connection state type parameter from `C` to
    /// `C2`. Subsequent builder methods will operate on the new connection
    /// state type. Note that any previously configured `on_connection_teardown`
    /// hook is cleared because its type signature depends on the old state
    /// type. The `on_error` hook is preserved since it does not depend on
    /// the connection state type.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// struct Session {
    ///     id: u64,
    /// }
    ///
    /// let builder =
    ///     WireframeClientBuilder::new().on_connection_setup(|| async { Session { id: 42 } });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_connection_setup<F, Fut, C2>(self, f: F) -> WireframeClientBuilder<S, P, C2>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = C2> + Send + 'static,
        C2: Send + 'static,
    {
        // Preserve on_error since it is not parameterized by C.
        // on_disconnect must be cleared because its signature depends on C.
        let on_error = self.lifecycle_hooks.on_error;
        builder_field_update!(
            self,
            lifecycle_hooks = LifecycleHooks {
                on_connect: Some(Arc::new(move || Box::pin(f()))),
                on_disconnect: None,
                on_error,
            }
        )
    }
}

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
{
    /// Register a callback invoked when the connection is closed.
    ///
    /// The callback receives the connection state produced by
    /// [`on_connection_setup`](Self::on_connection_setup). The teardown hook
    /// is invoked when [`WireframeClient::close`](crate::client::WireframeClient::close)
    /// is called.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// struct Session {
    ///     id: u64,
    /// }
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .on_connection_setup(|| async { Session { id: 42 } })
    ///     .on_connection_teardown(|session| async move {
    ///         println!("Session {} closed", session.id);
    ///     });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_connection_teardown<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.lifecycle_hooks.on_disconnect = Some(Arc::new(move |c| Box::pin(f(c))));
        self
    }

    /// Register a callback invoked when an error occurs.
    ///
    /// The callback receives a reference to the error and can perform logging
    /// or recovery actions. The handler is invoked before the error is returned
    /// to the caller.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().on_error(|err| async move {
    ///     eprintln!("Client error: {err}");
    /// });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_error<F, Fut>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a ClientError) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.lifecycle_hooks.on_error = Some(Arc::new(move |e| Box::pin(f(e))));
        self
    }
}
