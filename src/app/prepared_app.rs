//! Immutable application data prepared for connection handling.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::mpsc;

use super::{
    PrepareError,
    builder::WireframeApp,
    envelope::Packet,
    inbound_handler::build_route_chains,
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
};
use crate::{
    codec::{FrameCodec, LengthDelimitedFrameCodec, RecoveryConfig, RecoveryPolicyHook},
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    metrics::{self, PreparationOutcome},
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

mod connection;

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
    ///
    /// Shared so the connection actor's `'static` protocol hooks can decode
    /// terminator frames without copying the serializer.
    pub(in crate::app) serializer: Arc<S>,
    /// Codec template used to configure each connection's framed transport.
    pub(in crate::app) codec: F,
    /// Optional hook that creates per-connection state.
    pub(in crate::app) on_connect: Option<Arc<ConnectionSetup<C>>>,
    /// Optional hook that releases per-connection state after processing.
    pub(in crate::app) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    /// Optional protocol hook used to customize frame-level processing.
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional assembler for protocol messages spread across several frames.
    pub(in crate::app) message_assembler: Option<Arc<dyn MessageAssembler>>,
    // FIXME: Issue #643 keeps this template-owned configuration until the
    // connection-local runtime consumes it: https://github.com/leynos/wireframe/issues/643.
    #[expect(
        dead_code,
        reason = "FIXME: #643 tracks ConnectionRuntime consuming the push DLQ"
    )]
    /// Optional dead-letter sink for pushes that cannot be delivered.
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    /// Optional limits and timeout for transparent frame fragmentation.
    pub(in crate::app) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Maximum interval to wait for the next inbound frame.
    pub(in crate::app) read_timeout_ms: u64,
    /// Optional byte caps protecting message and connection memory usage.
    pub(in crate::app) memory_budgets: Option<MemoryBudgets>,
    /// Optional hook consulted when a decoder fails on a live connection.
    pub(in crate::app) recovery_hook: Option<Arc<dyn RecoveryPolicyHook>>,
    /// Global recovery settings copied into each connection.
    pub(in crate::app) recovery_config: RecoveryConfig,
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
        let routes = build_route_chains(&self.handlers, &self.middleware).await;

        Ok(PreparedApp {
            routes,
            serializer: self.serializer,
            codec: self.codec,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            protocol: self.protocol,
            message_assembler: self.message_assembler,
            push_dlq: self.push_dlq,
            fragmentation: self.fragmentation,
            read_timeout_ms: self.read_timeout_ms,
            memory_budgets: self.memory_budgets,
            recovery_hook: self.recovery_hook,
            recovery_config: self.recovery_config,
        })
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
        self.protocol.as_ref().map_or_else(
            crate::hooks::ProtocolHooks::default,
            crate::hooks::ProtocolHooks::from_protocol,
        )
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
