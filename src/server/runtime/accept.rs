//! Accept-loop utilities for server runtime.

use std::{fmt, io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use log::warn;
use tokio::{
    net::{TcpListener, TcpStream},
    select,
    time::{Duration, sleep},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::backoff::BackoffConfig;
use crate::{
    app::{Envelope, Packet},
    codec::FrameCodec,
    frame::FrameMetadata,
    preamble::Preamble,
    serializer::Serializer,
    server::{AppFactory, PreambleFailure, PreambleHandler, connection::spawn_connection_task},
};

/// Abstraction for sources of incoming connections consumed by the accept loop.
///
/// Implementations must be cancellation-safe: dropping a pending `accept()`
/// future must not leak resources.
#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub(in crate::server) trait AcceptListener: Send + Sync {
    async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)>;
    fn local_addr(&self) -> io::Result<SocketAddr>;
}

#[async_trait]
impl AcceptListener for TcpListener {
    async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        TcpListener::accept(self).await
    }

    fn local_addr(&self) -> io::Result<SocketAddr> { TcpListener::local_addr(self) }
}

#[derive(Debug)]
pub(in crate::server) struct AcceptLoopOptions<T> {
    pub preamble: PreambleHooks<T>,
    pub shutdown: CancellationToken,
    pub tracker: TaskTracker,
    pub backoff: BackoffConfig,
}

struct AcceptHandles<'a, T> {
    preamble: &'a PreambleHooks<T>,
    shutdown: &'a CancellationToken,
    tracker: &'a TaskTracker,
    backoff: &'a BackoffConfig,
}

#[derive(Default)]
pub(in crate::server) struct PreambleHooks<T> {
    pub on_success: Option<PreambleHandler<T>>,
    pub on_failure: Option<PreambleFailure>,
    pub timeout: Option<Duration>,
}

impl<T> Clone for PreambleHooks<T> {
    fn clone(&self) -> Self {
        Self {
            on_success: self.on_success.clone(),
            on_failure: self.on_failure.clone(),
            timeout: self.timeout,
        }
    }
}

impl<T> fmt::Debug for PreambleHooks<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreambleHooks")
            .field(
                "on_success",
                &self.on_success.as_ref().map(|_| "Some(<handler>)"),
            )
            .field(
                "on_failure",
                &self.on_failure.as_ref().map(|_| "Some(<failure>)"),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Accepts incoming connections and spawns handler tasks.
///
/// The loop accepts connections from `listener`, creates a new
/// [`WireframeApp`] via `factory` for each one, and spawns a task to handle
/// the connection. Failures to accept a connection trigger an exponential
/// back-off governed by `backoff_config`. The loop terminates when `shutdown`
/// is cancelled, and all spawned tasks are tracked by `tracker` for graceful
/// shutdown.
///
/// # Parameters
///
/// - `listener`: Source of incoming TCP connections.
/// - `factory`: [`AppFactory`] that creates a fresh [`WireframeApp`] for each connection.
/// - `preamble`: Preamble handlers and timeout configuration.
/// - `shutdown`: Signal used to stop the accept loop.
/// - `tracker`: Task tracker used for graceful shutdown.
/// - `backoff_config`: Controls exponential back-off behaviour.
///
/// # Type Parameters
///
/// - `F`: [`AppFactory`] implementation that creates [`WireframeApp`] instances.
/// - `T`: Preamble type for connection handshaking.
/// - `L`: Listener type implementing [`AcceptListener`].
///
/// # Examples
///
/// ```ignore
/// use std::sync::Arc;
///
/// use tokio_util::{sync::CancellationToken, task::TaskTracker};
/// use wireframe::{
///     app::WireframeApp,
///     // server::runtime::{AcceptListener, BackoffConfig, PreambleHooks, accept_loop},
/// };
///
/// async fn run<L: AcceptListener + Send + Sync + 'static>(listener: Arc<L>) {
///     let tracker = TaskTracker::new();
///     let token = CancellationToken::new();
///     accept_loop(
///         listener,
///         || WireframeApp::default(),
///         AcceptLoopOptions::<()> {
///             preamble: PreambleHooks::default(),
///             shutdown: token,
///             tracker,
///             backoff: BackoffConfig::default(),
///         },
///     )
///     .await;
/// }
/// ```
pub(in crate::server) async fn accept_loop<F, T, L, Ser, Ctx, E, Codec>(
    listener: Arc<L>,
    factory: F,
    options: AcceptLoopOptions<T>,
) where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    L: AcceptListener + Send + Sync + 'static,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
{
    let AcceptLoopOptions {
        preamble,
        shutdown,
        tracker,
        backoff,
    } = options;
    let backoff = backoff.normalized();
    debug_assert!(
        backoff.initial_delay <= backoff.max_delay,
        "BackoffConfig invariant violated: initial_delay > max_delay"
    );
    debug_assert!(
        backoff.initial_delay >= Duration::from_millis(1),
        "BackoffConfig invariant violated: initial_delay < 1ms"
    );
    let mut delay = backoff.initial_delay;
    let handles = AcceptHandles {
        preamble: &preamble,
        shutdown: &shutdown,
        tracker: &tracker,
        backoff: &backoff,
    };
    while let Some(next_delay) = accept_iteration(&listener, &factory, &handles, delay).await {
        delay = next_delay;
    }
}

#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! expands to modulus internally"
)]
async fn accept_iteration<F, T, L, Ser, Ctx, E, Codec>(
    listener: &Arc<L>,
    factory: &F,
    handles: &AcceptHandles<'_, T>,
    delay: Duration,
) -> Option<Duration>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    L: AcceptListener + Send + Sync + 'static,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
{
    select! {
        biased;

        () = handles.shutdown.cancelled() => None,
        res = listener.accept() => Some(match res {
            Ok((stream, _)) => {
                spawn_connection_task(
                    stream,
                    factory.clone(),
                    handles.preamble.clone(),
                    handles.tracker,
                );
                handles.backoff.initial_delay
            }
            Err(e) => {
                let local_addr = listener.local_addr().ok();
                warn!("accept error: error={e:?}, local_addr={local_addr:?}");
                sleep(delay).await;
                (delay * 2).min(handles.backoff.max_delay)
            }
        }),
    }
}
