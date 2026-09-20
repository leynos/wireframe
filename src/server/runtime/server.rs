//! Running-server ownership and terminal observation.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures::Future;
use log::warn;
use tokio::{
    net::TcpListener,
    sync::{oneshot, watch},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::Instrument;

use super::{
    AcceptLoopOptions,
    PreambleHooks,
    SupervisorLifecycle,
    accept_loop,
    backoff::BackoffConfig,
    startup::{prepare_application, prepare_application_or_shutdown},
    supervisor::{
        SupervisorCancellationDropGuard,
        await_supervisor_termination,
        run_with_panic_cleanup,
    },
};
use crate::{
    app::{Envelope, Packet, PreparedApp},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    metrics::{ServerStartupOutcome, record_server_startup_duration},
    preamble::Preamble,
    serializer::Serializer,
    server::{
        AppFactory,
        Bound,
        PreambleFailure,
        PreambleHandler,
        ServerError,
        ServerShutdown,
        WireframeServer,
    },
};

mod observer;
pub(super) use observer::observe_supervisor_termination;

/// State retained exclusively by the supervisor task after preparation.
struct PreparedServerRuntime<
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
> where
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    /// Number of accept loops owned by this supervisor.
    workers: usize,
    /// Handler run after a successfully decoded connection preamble.
    on_preamble_success: Option<PreambleHandler<T>>,
    /// Handler run when a connection preamble cannot be decoded.
    on_preamble_failure: Option<PreambleFailure>,
    /// Optional readiness notification sent after accept loops are spawned.
    ready_tx: Option<oneshot::Sender<()>>,
    /// Listener retained until every accept loop exits.
    listener: Arc<TcpListener>,
    /// Retry policy copied into each accept loop.
    backoff_config: BackoffConfig,
    /// Optional deadline for decoding a connection preamble.
    preamble_timeout: Option<Duration>,
    /// Application template shared by independent connection tasks.
    app: Arc<PreparedApp<Ser, Ctx, E, Codec>>,
}

/// State whose ownership must survive a supervisor panic long enough to drain.
struct SupervisorRuntime {
    /// Shared cancellation signal for accept loops and panic cleanup.
    shutdown_token: CancellationToken,
    /// Bounded cancellation reason shared with accept-loop observability.
    lifecycle: SupervisorLifecycle,
    /// Every accept and connection task that must drain before publication.
    tracker: TaskTracker,
}

impl SupervisorRuntime {
    /// Create one supervisor-owned cancellation and task-lifetime context.
    fn new(shutdown_token: CancellationToken) -> Self {
        Self {
            shutdown_token,
            lifecycle: SupervisorLifecycle::new(),
            tracker: TaskTracker::new(),
        }
    }
}

impl<T, Ser, Ctx, E, Codec> PreparedServerRuntime<T, Ser, Ctx, E, Codec>
where
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    /// Spawn accept loops, await their stop condition, and drain tracked work.
    async fn run_with_shutdown<S>(
        self,
        shutdown: S,
        supervisor: &SupervisorRuntime,
        startup_started: Instant,
    ) -> Result<(), ServerError>
    where
        S: Future<Output = ()>,
    {
        let preamble = PreambleHooks {
            on_success: self.on_preamble_success,
            on_failure: self.on_preamble_failure,
            timeout: self.preamble_timeout,
        };

        for _ in 0..self.workers {
            let listener = Arc::clone(&self.listener);
            let app = Arc::clone(&self.app);
            let preamble_hooks = preamble.clone();
            let token = supervisor.shutdown_token.clone();
            let tracker_clone = supervisor.tracker.clone();
            let span = tracing::Span::current();
            supervisor.tracker.spawn(
                accept_loop(
                    listener,
                    AcceptLoopOptions {
                        app,
                        preamble: preamble_hooks,
                        shutdown: token,
                        tracker: tracker_clone,
                        backoff: self.backoff_config,
                        lifecycle: supervisor.lifecycle.clone(),
                    },
                )
                .instrument(span),
            );
        }

        record_server_startup_duration(ServerStartupOutcome::Success, startup_started.elapsed());

        if let Some(tx) = self.ready_tx
            && tx.send(()).is_err()
        {
            warn!("Failed to send readiness signal: receiver dropped");
        }

        await_supervisor_termination(
            shutdown,
            &supervisor.shutdown_token,
            &supervisor.tracker,
            &supervisor.lifecycle,
        )
        .await;
        supervisor.tracker.close();
        supervisor.tracker.wait().await;
        Ok(())
    }
}

impl<F, T, Ser, Ctx, E, Codec> WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    /// Run the server until a shutdown signal is received.
    ///
    /// This preserves the original foreground lifecycle: the caller owns the
    /// supervisor future and Ctrl+C requests graceful shutdown.
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
    /// # Errors
    ///
    /// Returns typed application factory or preparation errors before the
    /// server begins accepting connections.
    pub async fn run(self) -> Result<(), ServerError> {
        self.run_with_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    }

    /// Run the server until the supplied shutdown future resolves.
    ///
    /// Dropping this future still cancels the accept loops. Existing
    /// connection tasks retain their established graceful-drain behaviour.
    ///
    /// # Errors
    ///
    /// Returns typed application factory or preparation errors before the
    /// server begins accepting connections.
    pub async fn run_with_shutdown<S>(self, shutdown: S) -> Result<(), ServerError>
    where
        S: Future<Output = ()> + Send,
    {
        let startup_started = Instant::now();
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
        let Some((app, shutdown)) =
            prepare_application_or_shutdown(factory, shutdown, startup_started).await?
        else {
            return Ok(());
        };
        let runtime = PreparedServerRuntime {
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            listener,
            backoff_config,
            preamble_timeout,
            app,
        };
        let supervisor = SupervisorRuntime::new(CancellationToken::new());
        // No worker exists until application preparation has succeeded.
        let _cancel_workers_on_drop = SupervisorCancellationDropGuard::new(
            supervisor.shutdown_token.drop_guard_ref(),
            supervisor.lifecycle.clone(),
        );
        runtime
            .run_with_shutdown(shutdown, &supervisor, startup_started)
            .await
    }

    /// Prepare and start the server, returning its shutdown control.
    ///
    /// Startup failures are reported before a control handle exists. Once this
    /// method succeeds, [`ServerShutdown::drained`] reports either a clean
    /// drain or an abnormal supervisor termination.
    ///
    /// # Errors
    ///
    /// Returns typed application factory or preparation errors before a
    /// running shutdown control is returned.
    pub async fn spawn(self) -> Result<ServerShutdown, ServerError> {
        let startup_started = Instant::now();
        let runtime = self.prepare_runtime(startup_started).await?;
        let shutdown_token = CancellationToken::new();
        let (terminal_tx, terminal_rx) = watch::channel(None);
        let supervisor_context = SupervisorRuntime::new(shutdown_token.clone());
        let shutdown = supervisor_context.shutdown_token.clone().cancelled_owned();
        let supervisor = tokio::spawn(async move {
            let run = runtime.run_with_shutdown(shutdown, &supervisor_context, startup_started);
            run_with_panic_cleanup(
                run,
                &supervisor_context.shutdown_token,
                &supervisor_context.tracker,
            )
            .await
        });

        // The observer exclusively owns the supervisor handle until it sends
        // the terminal outcome shared by all `ServerShutdown` clones.
        drop(tokio::spawn(observe_supervisor_termination(
            supervisor,
            terminal_tx,
        )));

        Ok(ServerShutdown::new(shutdown_token, terminal_rx))
    }

    /// Build and prepare the shared application before starting any worker.
    async fn prepare_runtime(
        self,
        startup_started: Instant,
    ) -> Result<PreparedServerRuntime<T, Ser, Ctx, E, Codec>, ServerError> {
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
        let app = prepare_application(factory, startup_started).await?;

        Ok(PreparedServerRuntime {
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            listener,
            backoff_config,
            preamble_timeout,
            app,
        })
    }
}

#[cfg(test)]
#[path = "server/observation_tests.rs"]
mod observation_tests;
