//! Bounded metrics for server start-up outcomes.

use std::time::Duration;

#[cfg(feature = "metrics")]
use ::metrics::{counter, histogram};

/// Name of the counter tracking server start-up failures.
///
/// The bounded `stage` label is either `"factory_build"` or `"preparation"`.
pub const SERVER_STARTUP_FAILURES: &str = "wireframe_server_startup_failures_total";

/// Name of the histogram recording server startup duration in seconds.
///
/// The bounded `outcome` label is either `"success"`, `"factory_build"`,
/// `"preparation"`, or `"cancelled"`.
pub const SERVER_STARTUP_DURATION: &str = "wireframe_server_startup_duration_seconds";

/// Bounded stages at which server start-up can fail.
#[derive(Clone, Copy)]
pub(crate) enum ServerStartupFailureStage {
    /// The startup factory did not produce an application.
    FactoryBuild,
    /// The application did not produce an immutable prepared template.
    Preparation,
}

impl ServerStartupFailureStage {
    /// Return the stable metric and tracing label value for this stage.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::FactoryBuild => "factory_build",
            Self::Preparation => "preparation",
        }
    }
}

/// Bounded outcomes for server startup duration metrics.
#[derive(Clone, Copy)]
pub(crate) enum ServerStartupOutcome {
    /// Workers were installed and the server may signal readiness.
    Success,
    /// The startup factory failed before producing an application.
    FactoryBuild,
    /// Application preparation failed before worker installation.
    Preparation,
    /// Shutdown completed while application preparation was still pending.
    Cancelled,
}

impl ServerStartupOutcome {
    /// Return the stable metric label value for this outcome.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::FactoryBuild => "factory_build",
            Self::Preparation => "preparation",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Record a server start-up failure at a bounded stage.
#[cfg(feature = "metrics")]
pub(crate) fn inc_server_startup_failure(stage: ServerStartupFailureStage) {
    counter!(SERVER_STARTUP_FAILURES, "stage" => stage.as_str()).increment(1);
}

/// Record total startup duration with a bounded outcome.
#[cfg(feature = "metrics")]
pub(crate) fn record_server_startup_duration(outcome: ServerStartupOutcome, elapsed: Duration) {
    histogram!(SERVER_STARTUP_DURATION, "outcome" => outcome.as_str())
        .record(elapsed.as_secs_f64());
}

/// Record total startup duration with a bounded outcome.
#[cfg(not(feature = "metrics"))]
pub(crate) fn record_server_startup_duration(_outcome: ServerStartupOutcome, _elapsed: Duration) {}

/// Record a server start-up failure at a bounded stage.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_server_startup_failure(_stage: ServerStartupFailureStage) {}
