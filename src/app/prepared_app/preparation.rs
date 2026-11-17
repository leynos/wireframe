//! Turning a [`WireframeApp`] builder into an immutable prepared template.
//!
//! Middleware transforms run once during this transition, so the resulting
//! [`PreparedApp`] can drive many connections without rebuilding its route
//! chains. The work lives in its own file to keep the template's data
//! definition small.

use std::time::{Duration, Instant};

use super::PreparedApp;
use crate::{
    app::{
        PrepareError,
        builder::WireframeApp,
        envelope::Packet,
        inbound_handler::build_route_chains,
    },
    codec::FrameCodec,
    metrics::{self, PreparationOutcome},
    serializer::Serializer,
};

/// Supplies elapsed durations for preparation instrumentation.
pub(super) trait PreparationTimeSource {
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
    pub(super) async fn prepare_with_time_source<T>(
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
