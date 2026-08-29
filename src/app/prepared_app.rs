//! Immutable application data prepared for connection handling.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::mpsc,
};
use tracing::Instrument as _;

use super::{
    PrepareError,
    builder::WireframeApp,
    envelope::Packet,
    inbound_handler::{ConnectionProcessingContext, build_route_chains, process_connection},
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
};
use crate::{
    app_data_store::AppDataStore,
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    frame::FrameMetadata,
    hooks::WireframeProtocol,
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    metrics::{self, PreparationOutcome},
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

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
    /// Serializer retained by every connection driven by this application.
    pub(in crate::app) serializer: S,
    /// Codec template used to configure each connection's framed transport.
    pub(in crate::app) codec: F,
    // Retain this template-owned state until the connection-local runtime in
    // https://github.com/leynos/wireframe/issues/643 consumes it.
    #[expect(
        dead_code,
        reason = "tracked by issue #643: ConnectionRuntime will consume application data"
    )]
    /// Type-erased application state retained for the connection-runtime slice.
    pub(in crate::app) app_data: AppDataStore,
    /// Optional hook that creates per-connection state.
    pub(in crate::app) on_connect: Option<Arc<ConnectionSetup<C>>>,
    /// Optional hook that releases per-connection state after processing.
    pub(in crate::app) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    /// Optional protocol hook used to customize frame-level processing.
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional assembler for protocol messages spread across several frames.
    pub(in crate::app) message_assembler: Option<Arc<dyn MessageAssembler>>,
    // Retain this template-owned configuration until the connection-local
    // runtime in https://github.com/leynos/wireframe/issues/643 consumes it.
    #[expect(
        dead_code,
        reason = "tracked by issue #643: ConnectionRuntime will consume the push DLQ"
    )]
    /// Optional dead-letter sink for pushes that cannot be delivered.
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
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

/// Supplies elapsed durations for prepared-connection instrumentation.
trait ConnectionTimeSource {
    /// Opaque point captured at the beginning of prepared connection handling.
    type StartedAt;

    /// Capture a point from which connection duration is measured.
    fn start(&self) -> Self::StartedAt;

    /// Return the elapsed duration since a captured connection point.
    fn elapsed(&self, started_at: Self::StartedAt) -> Duration;
}

/// Production time source backed by the monotonic standard-library clock.
struct SystemConnectionTimeSource;

impl ConnectionTimeSource for SystemConnectionTimeSource {
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
        let routes = build_route_chains(&self.handlers, &self.middleware).await;

        Ok(PreparedApp {
            routes,
            serializer: self.serializer,
            codec: self.codec,
            app_data: self.app_data,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            protocol: self.protocol,
            message_assembler: self.message_assembler,
            push_dlq: self.push_dlq,
            fragmentation: self.fragmentation,
            read_timeout_ms: self.read_timeout_ms,
            memory_budgets: self.memory_budgets,
        })
    }
}

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = super::Envelope> + Send + Sync,
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
        self.handle_connection_with_time_source(stream, &SystemConnectionTimeSource)
            .await
    }

    /// Handle a connection with an injectable instrumentation time source.
    async fn handle_connection_with_time_source<W, T>(
        &self,
        stream: W,
        time_source: &T,
    ) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
        T: ConnectionTimeSource,
    {
        metrics::inc_prepared_connection_uses();
        let started_at = time_source.start();
        let span = tracing::info_span!(
            "prepared_connection",
            outcome = tracing::field::Empty,
            elapsed_ms = tracing::field::Empty
        );
        let result = process_connection(
            stream,
            ConnectionProcessingContext {
                routes: &self.routes,
                serializer: &self.serializer,
                codec: &self.codec,
                on_connect: self.on_connect.as_ref(),
                on_disconnect: self.on_disconnect.as_ref(),
                message_assembler: self.message_assembler.as_ref(),
                fragmentation: self.fragmentation,
                memory_budgets: self.memory_budgets,
                read_timeout_ms: self.read_timeout_ms,
            },
        )
        .instrument(span.clone())
        .await;
        span.record("outcome", if result.is_ok() { "success" } else { "error" });
        let elapsed_ms =
            u64::try_from(time_source.elapsed(started_at).as_millis()).unwrap_or(u64::MAX);
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

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Get a clone of the configured protocol, if any.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// assert!(prepared.protocol().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn protocol(
        &self,
    ) -> Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>> {
        self.protocol.clone()
    }

    /// Return protocol hooks derived from the installed protocol.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let hooks = prepared.protocol_hooks();
    /// # let _ = hooks;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn protocol_hooks(&self) -> crate::hooks::ProtocolHooks<F::Frame, ()> {
        self.protocol
            .as_ref()
            .map(crate::hooks::ProtocolHooks::from_protocol)
            .unwrap_or_default()
    }

    /// Get the configured message assembler, if any.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// assert!(prepared.message_assembler().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn message_assembler(&self) -> Option<&Arc<dyn MessageAssembler>> {
        self.message_assembler.as_ref()
    }
}

#[cfg(test)]
#[path = "prepared_app_tests.rs"]
mod tests;
