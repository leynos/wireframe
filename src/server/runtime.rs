//! Runtime control for [`WireframeServer`].

mod accept;
mod backoff;
#[cfg(test)]
mod tests;

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[cfg(test)]
pub(super) use accept::MockAcceptListener;
pub(super) use accept::{AcceptLoopOptions, PreambleHooks, accept_loop};
pub use backoff::BackoffConfig;
use futures::Future;
use log::warn;
use tokio::{select, signal};
use tokio_util::{
    sync::{CancellationToken, DropGuardRef},
    task::TaskTracker,
};
use tracing::Instrument;

use super::{AppFactory, Bound, ServerError, WireframeServer};
use crate::{
    app::{Envelope, Packet},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    metrics::{ServerCancellationReason, inc_server_supervisor_cancellation},
    preamble::Preamble,
    serializer::Serializer,
};

const SUPERVISOR_RUNNING: u8 = 0;
const SUPERVISOR_GRACEFUL: u8 = 1;
const SUPERVISOR_DROPPED: u8 = 2;
const SUPERVISOR_FINISHED: u8 = 3;

/// Shares one terminal cancellation reason with the supervisor's accept loops.
///
/// The state only outlives the supervisor while an accept loop needs it to
/// record its own exit. It never owns a listener or task tracker.
#[derive(Clone, Debug)]
pub(in crate::server) struct SupervisorLifecycle {
    state: Arc<AtomicU8>,
}

impl SupervisorLifecycle {
    fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(SUPERVISOR_RUNNING)),
        }
    }

    fn record_graceful_cancellation(&self) {
        self.state.store(SUPERVISOR_GRACEFUL, Ordering::Release);
        record_supervisor_cancellation(ServerCancellationReason::Graceful);
    }

    fn record_dropped_cancellation(&self) {
        if self
            .state
            .compare_exchange(
                SUPERVISOR_RUNNING,
                SUPERVISOR_DROPPED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            record_supervisor_cancellation(ServerCancellationReason::Dropped);
        }
    }

    fn record_completion_without_cancellation(&self) {
        self.state.store(SUPERVISOR_FINISHED, Ordering::Release);
    }

    pub(in crate::server) fn cancellation_reason(&self) -> ServerCancellationReason {
        match self.state.load(Ordering::Acquire) {
            SUPERVISOR_GRACEFUL => ServerCancellationReason::Graceful,
            SUPERVISOR_DROPPED | SUPERVISOR_RUNNING | SUPERVISOR_FINISHED => {
                ServerCancellationReason::Dropped
            }
            _ => ServerCancellationReason::Dropped,
        }
    }
}

fn record_supervisor_cancellation(reason: ServerCancellationReason) {
    inc_server_supervisor_cancellation(reason);
    tracing::info!(
        event = "server_supervisor_cancellation",
        reason = reason.as_str(),
        "server_supervisor_cancellation",
    );
}

/// Cancels accept loops when its supervisor frame is abandoned.
///
/// The wrapped [`DropGuardRef`] preserves Tokio's borrowed cancellation guard;
/// this wrapper records the terminal reason before that guard cancels workers.
struct SupervisorCancellationDropGuard<'a> {
    _guard: DropGuardRef<'a>,
    lifecycle: SupervisorLifecycle,
}

impl<'a> SupervisorCancellationDropGuard<'a> {
    fn new(guard: DropGuardRef<'a>, lifecycle: SupervisorLifecycle) -> Self {
        Self {
            _guard: guard,
            lifecycle,
        }
    }
}

impl Drop for SupervisorCancellationDropGuard<'_> {
    fn drop(&mut self) { self.lifecycle.record_dropped_cancellation(); }
}

#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! expands to modulus internally"
)]
async fn await_supervisor_termination<S>(
    shutdown: S,
    shutdown_token: &CancellationToken,
    tracker: &TaskTracker,
    lifecycle: &SupervisorLifecycle,
) where
    S: Future<Output = ()>,
{
    select! {
        () = shutdown => {
            lifecycle.record_graceful_cancellation();
            shutdown_token.cancel();
        }
        () = tracker.wait() => lifecycle.record_completion_without_cancellation(),
    }
}

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
        let lifecycle = SupervisorLifecycle::new();
        // Cancel workers if this future is dropped, for example via JoinHandle::abort.
        let _cancel_workers_on_drop = SupervisorCancellationDropGuard::new(
            shutdown_token.drop_guard_ref(),
            lifecycle.clone(),
        );
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
            let span = tracing::Span::current();
            tracker.spawn(
                accept_loop(
                    listener,
                    factory,
                    AcceptLoopOptions {
                        preamble: preamble_hooks,
                        shutdown: token,
                        tracker: t,
                        backoff: backoff_config,
                        lifecycle: lifecycle.clone(),
                    },
                )
                .instrument(span),
            );
        }

        // Signal readiness after all workers have been spawned.
        if let Some(tx) = ready_tx
            && tx.send(()).is_err()
        {
            warn!("Failed to send readiness signal: receiver dropped");
        }

        await_supervisor_termination(shutdown, &shutdown_token, &tracker, &lifecycle).await;

        tracker.close();
        tracker.wait().await;
        Ok(())
    }
}
