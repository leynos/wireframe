//! Connection lifecycle builder methods for [`WireframeApp`].

use std::{future::Future, sync::Arc};

use super::{builder::WireframeApp, envelope::Packet, error::Result};
use crate::{codec::FrameCodec, serializer::Serializer};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Register a callback invoked when a new connection is established.
    ///
    /// The callback can perform authentication or other setup tasks and
    /// returns connection-specific state stored for the connection's
    /// lifetime.
    ///
    /// # Type Parameters
    ///
    /// This method changes the connection state type parameter from `C` to `C2`.
    /// This means that any subsequent builder methods will operate on the new connection state
    /// type `C2`. Be aware of this type transition when chaining builder methods.
    ///
    /// # Errors
    ///
    /// This function always succeeds currently but uses [`Result`] for
    /// consistency with other builder methods.
    pub fn on_connection_setup<SetupFn, Fut, C2>(
        self,
        f: SetupFn,
    ) -> Result<WireframeApp<S, C2, E, F>>
    where
        SetupFn: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = C2> + Send + 'static,
        C2: Send + 'static,
    {
        Ok(self.rebuild_with_connection_type(Some(Arc::new(move || Box::pin(f()))), None))
    }

    /// Register a callback invoked when a connection is closed.
    ///
    /// The callback receives the connection state produced by
    /// [`on_connection_setup`](Self::on_connection_setup).
    ///
    /// # Errors
    ///
    /// This function always succeeds currently but uses [`Result`] for
    /// consistency with other builder methods.
    pub fn on_connection_teardown<TeardownFn, Fut>(mut self, f: TeardownFn) -> Result<Self>
    where
        TeardownFn: Fn(C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.on_disconnect = Some(Arc::new(move |c| Box::pin(f(c))));
        Ok(self)
    }
}
