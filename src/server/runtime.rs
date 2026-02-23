//! Runtime control for [`WireframeServer`].

mod accept;
mod backoff;
#[cfg(test)]
mod tests;

use std::sync::Arc;

#[cfg(test)]
pub(super) use accept::MockAcceptListener;
pub(super) use accept::{AcceptLoopOptions, PreambleHooks, accept_loop};
pub use backoff::BackoffConfig;
use futures::Future;
use log::warn;
use tokio::{select, signal};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{AppFactory, Bound, ServerError, WireframeServer};
use crate::{
    app::{Envelope, Packet},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    preamble::Preamble,
    serializer::Serializer,
};

impl<F, T, Ser, Ctx, E, Codec> WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    /// Run the server until a shutdown signal is received.
    ///
    /// Spawns the configured number of worker tasks and awaits Ctrl+C for shutdown.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), wireframe::server::ServerError> {
    /// let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///     .bind(([127, 0, 0, 1], 8080).into())?;
    /// server.run().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Attempting to run a server without binding fails to compile:
    ///
    /// Binding specifies the network address for the server to listen on.
    /// It is required so the server knows where to accept incoming connections.
    ///
    /// ```compile_fail
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// async fn try_run() {
    ///     WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///         .run()
    ///         .await
    ///         .expect("unbound servers do not expose run()");
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if the server was not bound to a listener.
    /// Accept failures are retried with exponential back-off and do not
    /// surface as errors.
    pub async fn run(self) -> Result<(), ServerError> {
        self.run_with_shutdown(async {
            let _ = signal::ctrl_c().await;
        })
        .await
    }

    /// Run the server until the `shutdown` future resolves.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::sync::oneshot;
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), wireframe::server::ServerError> {
    /// let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///     .bind(([127, 0, 0, 1], 0).into())?;
    ///
    /// let (tx, rx) = oneshot::channel::<()>();
    /// let handle = tokio::spawn(async move {
    ///     server
    ///         .run_with_shutdown(async {
    ///             let _ = rx.await;
    ///         })
    ///         .await
    /// });
    ///
    /// // Signal shutdown
    /// let _ = tx.send(());
    /// handle
    ///     .await
    ///     .expect("join server task")
    ///     .expect("server run failed");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Attempting to run a server without binding fails to compile:
    ///
    /// Binding specifies the network address for the server to listen on.
    /// It is required so the server knows where to accept incoming connections.
    ///
    /// ```compile_fail
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// async fn try_run_with_shutdown() {
    ///     WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///         .run_with_shutdown(async {})
    ///         .await
    ///         .expect("unbound servers do not expose run_with_shutdown()");
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if the server was not bound to a listener.
    /// Accept failures are retried with exponential back-off and do not
    /// surface as errors.
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus internally"
    )]
    pub async fn run_with_shutdown<S>(self, shutdown: S) -> Result<(), ServerError>
    where
        S: Future<Output = ()> + Send,
    {
        let WireframeServer {
            factory,
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            state: Bound { listener },
            backoff_config,
            preamble_timeout,
            ..
        } = self;
        let shutdown_token = CancellationToken::new();
        let tracker = TaskTracker::new();
        let preamble = PreambleHooks {
            on_success: on_preamble_success,
            on_failure: on_preamble_failure,
            timeout: preamble_timeout,
        };

        for _ in 0..workers {
            let listener = Arc::clone(&listener);
            let factory = factory.clone();
            let preamble_hooks = preamble.clone();
            let token = shutdown_token.clone();
            let t = tracker.clone();
            tracker.spawn(accept_loop(
                listener,
                factory,
                AcceptLoopOptions {
                    preamble: preamble_hooks,
                    shutdown: token,
                    tracker: t,
                    backoff: backoff_config,
                },
            ));
        }

        // Signal readiness after all workers have been spawned.
        if let Some(tx) = ready_tx
            && tx.send(()).is_err()
        {
            warn!("Failed to send readiness signal: receiver dropped");
        }

        select! {
            () = shutdown => shutdown_token.cancel(),
            () = tracker.wait() => {},
        }

        tracker.close();
        tracker.wait().await;
        Ok(())
    }
}
