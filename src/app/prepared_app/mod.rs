//! Immutable application data prepared for connection handling.
//!
//! [`PreparedApp`] holds route services whose middleware chains were built once
//! by [`WireframeApp::prepare`], plus the streaming handlers, serializer,
//! codec, hooks, assembler, and budgets that every connection from it shares.
//! [`PreparedApp::handle_connection_result`] and
//! [`PreparedApp::handle_connection`] drive an accepted connection over those
//! services, and the accessors expose the protocol, its hooks, and the
//! assembler.
//!
//! Connection processing itself is in
//! [`crate::app::inbound_handler::process_connection`]. Streaming handlers are
//! carried without middleware transformation, so a streaming route never
//! enters a prepared service chain.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use monotony::{MonotonicClock, StdMonotonicClock};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::{
    PrepareError,
    builder::{
        WireframeApp,
        push::{validate_push_queues, warn_about_legacy_push_dlq},
    },
    envelope::Packet,
    inbound_handler::{ConnectionProcessingContext, build_route_chains_owned, process_connection},
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
};
use crate::{
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    frame::FrameMetadata,
    hooks::WireframeProtocol,
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    metrics::{self, PreparationOutcome},
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

mod accessors;

/// An immutable application template with fully transformed route services.
///
/// Obtain this type by consuming a [`WireframeApp`] with
/// [`WireframeApp::prepare`]. It deliberately has no route-registration API,
/// so all handler and middleware transforms have completed before connections
/// start using the application.
pub struct PreparedApp<
    S: Serializer + Send + Sync = BincodeSerializer,
    C: Send + 'static = (),
    E: Packet = super::Envelope,
    F: FrameCodec = LengthDelimitedFrameCodec,
> {
    /// Fully transformed route services keyed by protocol message identifier.
    pub(in crate::app) routes: HashMap<u32, HandlerService<E>>,
    /// Streaming handlers, which bypass middleware transformation.
    pub(in crate::app) streaming_routes: HashMap<u32, super::StreamingHandler>,
    /// Serializer retained by every connection driven by this application.
    ///
    /// Owned by the prepared root and borrowed as `&S` for the connection
    /// lifetime; the root outlives every connection, so no reference count is
    /// needed here.
    pub(in crate::app) serializer: Box<S>,
    /// Codec template used to configure each connection's framed transport.
    pub(in crate::app) codec: F,
    /// Optional hook that creates per-connection state.
    ///
    /// Borrowed through the prepared root for one connection lifecycle and
    /// never cloned per connection.
    pub(in crate::app) on_connect: Option<Box<ConnectionSetup<C>>>,
    /// Optional hook that releases per-connection state after processing.
    ///
    /// Borrowed through the prepared root for one connection lifecycle and
    /// never cloned per connection.
    pub(in crate::app) on_disconnect: Option<Box<ConnectionTeardown<C>>>,
    /// Optional protocol hook used to customize frame-level processing.
    ///
    /// Retained behind `Arc` because [`PreparedApp::protocol`] hands out a
    /// cloneable handle and #644 gives each connection exactly one strong
    /// reference through `ProtocolHooks`. See the ownership inventory in the
    /// developers' guide.
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional assembler for protocol messages spread across several frames.
    ///
    /// Owned by the prepared root and borrowed as `&dyn MessageAssembler`; a
    /// connection keeps only its own assembly state.
    pub(in crate::app) message_assembler: Option<Box<dyn MessageAssembler>>,
    // FIXME: Issue #643 keeps this template-owned configuration until the
    // connection-local runtime consumes it: https://github.com/leynos/wireframe/issues/643.
    #[expect(
        dead_code,
        reason = "FIXME: #643 tracks ConnectionRuntime consuming the push DLQ"
    )]
    /// Optional dead-letter sink for pushes that cannot be delivered.
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    /// Template cloned to build each connection's push queues.
    pub(in crate::app) push_queues: Option<crate::push::PushQueuesBuilder<F::Frame>>,
    /// Registry that each connection registers its push handle with.
    pub(in crate::app) session_registry: Arc<crate::session::SessionRegistry<F::Frame>>,
    /// Optional limits and timeout for transparent frame fragmentation.
    pub(in crate::app) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Maximum interval to wait for the next inbound frame.
    pub(in crate::app) read_timeout_ms: u64,
    /// Optional byte caps protecting message and connection memory usage.
    pub(in crate::app) memory_budgets: Option<MemoryBudgets>,
}

/// Supplies elapsed durations for preparation instrumentation.
trait PreparationTimeSource {
    /// Opaque point captured at the beginning of a preparation transition.
    type StartedAt;

    /// Capture a point from which preparation duration is measured.
    fn start(&self) -> Self::StartedAt;

    /// Return the elapsed duration since a captured preparation point.
    fn elapsed(&self, started_at: Self::StartedAt) -> Duration;
}

/// Production time source backed by the monotonic standard-library clock.
struct SystemPreparationTimeSource;

impl PreparationTimeSource for SystemPreparationTimeSource {
    type StartedAt = Instant;

    fn start(&self) -> Self::StartedAt { Instant::now() }

    fn elapsed(&self, started_at: Self::StartedAt) -> Duration { started_at.elapsed() }
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Consume builder registrations and prepare immutable route services.
    ///
    /// Middleware transforms run once for every registered route during this
    /// transition. The returned template can then drive multiple connections
    /// without rebuilding its route chains.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// # let _ = prepared;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`PrepareError`] if a future fallible preparation step fails.
    pub async fn prepare(self) -> Result<PreparedApp<S, C, E, F>, PrepareError> {
        self.prepare_with_time_source(&SystemPreparationTimeSource)
            .await
    }

    /// Prepare the application with an injectable instrumentation time source.
    async fn prepare_with_time_source<T>(
        self,
        time_source: &T,
    ) -> Result<PreparedApp<S, C, E, F>, PrepareError>
    where
        T: PreparationTimeSource,
    {
        let started_at = time_source.start();
        let result = self.build_prepared().await;
        let outcome = if result.is_ok() {
            PreparationOutcome::Success
        } else {
            PreparationOutcome::Failure
        };
        metrics::record_application_preparation(outcome, time_source.elapsed(started_at));
        result
    }

    /// Build the prepared representation before publishing it to the caller.
    async fn build_prepared(self) -> Result<PreparedApp<S, C, E, F>, PrepareError> {
        validate_push_queues(self.push_queues.as_ref())?;
        warn_about_legacy_push_dlq(self.push_dlq.as_ref());
        let Self {
            handlers,
            middleware,
            serializer,
            codec,
            on_connect,
            on_disconnect,
            protocol,
            streaming_handlers,
            push_dlq,
            push_queues,
            session_registry,
            fragmentation,
            message_assembler,
            read_timeout_ms,
            memory_budgets,
        } = self;
        // Preparation consumes the builder, so each handler's existing `Arc`
        // moves into its prepared service rather than being cloned. The
        // middleware list stays alive until every transform has run.
        let routes = build_route_chains_owned(handlers, &middleware).await;

        Ok(PreparedApp {
            routes,
            streaming_routes: streaming_handlers,
            serializer,
            codec,
            on_connect,
            on_disconnect,
            protocol,
            message_assembler,
            push_dlq,
            push_queues,
            session_registry,
            fragmentation,
            read_timeout_ms,
            memory_budgets,
        })
    }
}

/// Inputs scoped to one connection attempt rather than the whole application.
///
/// Grouped because they travel together from the accept loop to the connection
/// processing path, and a positional list of that length is easy to pass in the
/// wrong order.
struct ConnectionAttempt<'a> {
    /// Clock used to measure the connection's duration.
    clock: &'a dyn MonotonicClock,
    /// Token whose cancellation stops this connection.
    shutdown: &'a CancellationToken,
    /// Peer address the accept loop resolved, when it has one.
    peer_addr: Option<SocketAddr>,
}

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = super::Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    super::Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Handle an accepted connection using the prepared route services.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// prepared.handle_connection_result(server).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    pub async fn handle_connection_result<W>(&self, stream: W) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.handle_connection_with_shutdown(stream, CancellationToken::new(), None)
            .await
    }

    /// Handle an accepted connection, stopping it when `shutdown` is cancelled.
    ///
    /// The server passes its worker shutdown token here so a connection whose
    /// peer stopped reading cannot pin a graceful drain: its pending write is
    /// abandoned, which closes the connection. It also passes the peer address
    /// the accept loop already resolved, so connection-scoped diagnostics name
    /// the same peer the preamble hooks were given. Callers outside the server
    /// keep using [`Self::handle_connection_result`], whose token is never
    /// cancelled and which has no peer address to report.
    pub(crate) async fn handle_connection_with_shutdown<W>(
        &self,
        stream: W,
        shutdown: CancellationToken,
        peer_addr: Option<SocketAddr>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        self.handle_connection_inner(
            stream,
            ConnectionAttempt {
                clock: &StdMonotonicClock,
                shutdown: &shutdown,
                peer_addr,
            },
        )
        .await
    }

    /// Handle a connection with an injectable clock and shutdown signal.
    async fn handle_connection_inner<W>(
        &self,
        stream: W,
        attempt: ConnectionAttempt<'_>,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let ConnectionAttempt {
            clock,
            shutdown,
            peer_addr,
        } = attempt;
        metrics::inc_prepared_connection_uses();
        let started_at = clock.now();
        let span = tracing::info_span!(
            "prepared_connection",
            outcome = tracing::field::Empty,
            elapsed_ms = tracing::field::Empty
        );
        let result = process_connection(
            stream,
            ConnectionProcessingContext {
                routes: &self.routes,
                streaming_routes: &self.streaming_routes,
                serializer: self.serializer.as_ref(),
                codec: &self.codec,
                on_connect: self.on_connect.as_deref(),
                on_disconnect: self.on_disconnect.as_deref(),
                message_assembler: self.message_assembler.as_deref(),
                fragmentation: self.fragmentation,
                protocol_hooks: self.protocol_hooks(),
                memory_budgets: self.memory_budgets,
                read_timeout_ms: self.read_timeout_ms,
                push_queues: self.push_queues(),
                session_registry: Arc::clone(&self.session_registry),
                peer_addr,
                shutdown,
            },
        )
        .instrument(span.clone())
        .await;
        span.record("outcome", if result.is_ok() { "success" } else { "error" });
        let elapsed_ms =
            u64::try_from(clock.now().duration_since(started_at).as_millis()).unwrap_or(u64::MAX);
        span.record("elapsed_ms", elapsed_ms);
        result
    }

    /// Handle an accepted connection and log any processing failure.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// prepared.handle_connection(server).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle_connection<W>(&self, stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Err(error) = self.handle_connection_result(stream).await {
            log::warn!(
                "connection handling completed with error: correlation_id={:?}, error={error:?}",
                None::<u64>
            );
        }
    }
}

#[cfg(test)]
#[path = "prepared_app_tests.rs"]
mod tests;
