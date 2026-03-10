//! Managed pooled-connection wrapper.
//!
//! The wrapper tracks whether a socket should be discarded when it returns to
//! `bb8` and guarantees connection teardown hooks still run when the pooled
//! connection is dropped or reaped.

use std::{
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};

use tokio::{net::TcpStream, runtime::Handle};

use crate::{client::WireframeClient, rewind_stream::RewindStream, serializer::Serializer};

/// One physical client connection managed by `bb8`.
pub(crate) struct ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    client: ManuallyDrop<WireframeClient<S, RewindStream<TcpStream>, C>>,
    is_broken: bool,
}

impl<S, C> ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) fn new(client: WireframeClient<S, RewindStream<TcpStream>, C>) -> Self {
        Self {
            client: ManuallyDrop::new(client),
            is_broken: false,
        }
    }

    pub(crate) fn mark_broken(&mut self) { self.is_broken = true; }

    pub(crate) const fn is_broken(&self) -> bool { self.is_broken }
}

impl<S, C> Deref for ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    type Target = WireframeClient<S, RewindStream<TcpStream>, C>;

    fn deref(&self) -> &Self::Target { &self.client }
}

impl<S, C> DerefMut for ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.client }
}

impl<S, C> Drop for ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    fn drop(&mut self) {
        // SAFETY: `client` is valid here; `ManuallyDrop::take` is called
        // exactly once, in `Drop`, before the memory is freed.
        let client = unsafe { ManuallyDrop::take(&mut self.client) };

        if let Ok(handle) = Handle::try_current() {
            handle.spawn(async move {
                client.close().await;
            });
            return;
        }

        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(client.close());
        }
    }
}
