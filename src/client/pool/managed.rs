//! Managed pooled-connection wrapper.
//!
//! The wrapper tracks whether a socket should be discarded when it returns to
//! `bb8` and guarantees connection teardown hooks still run when the pooled
//! connection is dropped or reaped.

#[cfg(test)]
use tokio::sync::oneshot;
use tokio::{net::TcpStream, runtime::Handle};

use crate::{
    client::{ClientError, WireframeClient},
    rewind_stream::RewindStream,
    serializer::Serializer,
};

/// Complete a test wait only after the close future has returned.
#[cfg(test)]
fn signal_close_completion(completion_sender: Option<oneshot::Sender<()>>) {
    if let Some(sender) = completion_sender {
        sender.send(()).unwrap_or_default();
    }
}

/// One physical client connection managed by `bb8`.
pub(crate) struct ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Owned client, taken once when teardown starts.
    client: Option<WireframeClient<S, RewindStream<TcpStream>, C>>,
    /// Set after protocol or I/O failure to force pool replacement.
    is_broken: bool,
    /// Signals tests only after the detached close future completes.
    #[cfg(test)]
    close_completion: Option<oneshot::Sender<()>>,
}

impl<S, C> ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Wrap a connected client as a healthy pooled resource.
    pub(crate) const fn new(client: WireframeClient<S, RewindStream<TcpStream>, C>) -> Self {
        Self {
            client: Some(client),
            is_broken: false,
            #[cfg(test)]
            close_completion: None,
        }
    }

    /// Install a test-only signal for completion of the detached close task.
    #[cfg(test)]
    pub(crate) fn notify_after_close(&mut self, sender: oneshot::Sender<()>) {
        self.close_completion = Some(sender);
    }

    /// Mark the resource so `bb8` discards it instead of reusing it.
    pub(crate) const fn mark_broken(&mut self) { self.is_broken = true; }

    /// Report whether a prior operation invalidated this physical connection.
    pub(crate) const fn is_broken(&self) -> bool { self.is_broken }

    /// Borrow the live client while the resource is checked out of the pool.
    ///
    /// # Errors
    ///
    /// Returns a disconnection error if teardown has already taken the client.
    pub(crate) fn client_mut(
        &mut self,
    ) -> Result<&mut WireframeClient<S, RewindStream<TcpStream>, C>, ClientError> {
        self.client.as_mut().ok_or_else(ClientError::disconnected)
    }
}

impl<S, C> Drop for ManagedClientConnection<S, C>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
{
    fn drop(&mut self) {
        let Some(client) = self.client.take() else {
            return;
        };

        if let Ok(handle) = Handle::try_current() {
            #[cfg(test)]
            let close_completion = self.close_completion.take();
            handle.spawn(async move {
                client.close().await;
                #[cfg(test)]
                signal_close_completion(close_completion);
            });
            return;
        }

        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(client.close());
            #[cfg(test)]
            signal_close_completion(self.close_completion.take());
        }
    }
}
