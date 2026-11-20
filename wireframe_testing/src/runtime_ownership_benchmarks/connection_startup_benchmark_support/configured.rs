//! A startup workload whose application installs every configurable component.
//!
//! [`StartupWorkload`] measures the route and middleware chains, which is the
//! part of preparation #639 already covered. Issue #648 changes how the other
//! application-owned components are held — boxed serializer, boxed message
//! assembler, boxed lifecycle callbacks, and a shared protocol — so a baseline
//! that only builds bare routes cannot show that change's effect.
//!
//! This module therefore measures the same two topologies on an application
//! that also installs a message assembler, a protocol, and a setup/teardown
//! callback pair. The workload is a separate type rather than extra fields on
//! [`StartupWorkload`]: existing baselines name their work with the old label,
//! and adding components to it would silently redefine every recorded row.
//!
//! # The drop boundary
//!
//! Both phases include the prepared root's drop inside the measured region. A
//! preparation that allocates a boxed component allocates it once, but the
//! value is released when the prepared root drops, and a measurement that left
//! that out would attribute the cost to a later, unrelated scope. Every
//! iteration therefore drops its prepared root explicitly, and
//! [`measure_configured_startup`] states that in its documentation.
//!
//! # Phases
//!
//! [`ConfiguredPhase::PreparedThenServe`] prepares once and serves the
//! workload's connections from that root. [`ConfiguredPhase::PreparationOnly`]
//! builds, prepares, and drops the application without serving anything, which
//! isolates the cost #648 actually changes. Preparation-only is restricted to
//! the prepared-root topology: preparing per connection without connecting is
//! not a shape any caller uses, so measuring it would only add a misleading
//! row.

use bytes::Bytes;
use wireframe::{
    app::{Envelope, WireframeApp},
    hooks::{ConnectionContext, WireframeProtocol},
    push::PushHandle,
    serializer::BincodeSerializer,
    test_helpers::{
        TestAssembler,
        connection_startup::{
            ConnectionStartupCounts,
            ConnectionStartupInstrumentation,
            StartupFixturePlan,
            counted_app_factory,
        },
    },
};

use super::workload::{StartupMeasurement, StartupPath, StartupWorkload};

/// Configuration the configured workload installs, as it appears in labels.
///
/// Kept as one constant so the label a baseline records and the application
/// this module actually builds cannot drift apart: both refer to the same
/// statement of what is installed.
const CONFIGURATION_TAG: &str = "serializer_1_assembler_1_callbacks_2_protocol_1";

/// Which half of the configured workload a phase measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfiguredPhase {
    /// Prepare once, serve every connection, then drop the root.
    PreparedThenServe,
    /// Build, prepare, and drop the application without serving a connection.
    PreparationOnly,
}

/// Per-connection state produced by the configured setup callback.
///
/// Opaque on purpose: a benchmark must not let the compiler prove the state
/// unused, but nothing measures it beyond its existence.
#[derive(Debug)]
pub struct ConfiguredConnectionState {
    /// Identity handed to teardown so the value cannot be optimized away.
    id: u64,
}

impl ConfiguredConnectionState {
    /// Identity this state was created with.
    #[must_use]
    pub const fn id(&self) -> u64 { self.id }
}

impl Drop for ConfiguredConnectionState {
    fn drop(&mut self) { std::hint::black_box(self.id); }
}

/// Concrete application type built by the configured workload.
pub type ConfiguredStartupTestApp =
    WireframeApp<BincodeSerializer, ConfiguredConnectionState, Envelope>;

/// Protocol installed by the configured workload.
///
/// The default hook implementations are deliberate: the measured claim is that
/// an installed protocol costs one shared reference per connection, not that
/// any particular hook is cheap.
struct ConfiguredProtocol;

impl WireframeProtocol for ConfiguredProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn on_connection_setup(&self, _handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {}
}

/// One configured startup scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfiguredStartupWorkload {
    /// The route/middleware/connection parameters to measure.
    pub base: StartupWorkload,
    /// Which half of the scenario to measure.
    pub phase: ConfiguredPhase,
}

impl Default for ConfiguredStartupWorkload {
    fn default() -> Self {
        Self {
            base: StartupWorkload::default(),
            phase: ConfiguredPhase::PreparedThenServe,
        }
    }
}

impl ConfiguredStartupWorkload {
    /// The fixture plan this workload builds.
    #[must_use]
    pub const fn plan(self) -> StartupFixturePlan { self.base.plan() }

    /// Connections served per iteration, which is zero when preparing only.
    #[must_use]
    pub const fn served_connections(self) -> usize {
        match self.phase {
            ConfiguredPhase::PreparedThenServe => self.base.connections,
            ConfiguredPhase::PreparationOnly => 0,
        }
    }

    /// Benchmark label naming the installed components and the base workload.
    ///
    /// The preparation-only phase carries its own connection-free tail rather
    /// than reusing the base label: the base label states how many connections
    /// were served, and this phase serves none, so reusing it would put a
    /// figure in the baseline table that no workload produced.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{
    ///     ConfiguredPhase,
    ///     ConfiguredStartupWorkload,
    ///     StartupPath,
    ///     StartupWorkload,
    /// };
    ///
    /// let workload = ConfiguredStartupWorkload {
    ///     base: StartupWorkload {
    ///         path: StartupPath::PreparedRoot,
    ///         routes: 2,
    ///         middleware_layers: 2,
    ///         connections: 4,
    ///     },
    ///     phase: ConfiguredPhase::PreparedThenServe,
    /// };
    /// assert_eq!(
    ///     workload.label(),
    ///     concat!(
    ///         "configured_serializer_1_assembler_1_callbacks_2_protocol_1_",
    ///         "prepared_root_routes_2_layers_2_connections_4_",
    ///         "factory_1_transforms_4",
    ///     )
    /// );
    /// ```
    #[must_use]
    pub fn label(self) -> String {
        match self.phase {
            ConfiguredPhase::PreparedThenServe => {
                format!("configured_{CONFIGURATION_TAG}_{}", self.base.label())
            }
            ConfiguredPhase::PreparationOnly => format!(
                concat!(
                    "configured_{}_preparation_only_prepared_root_routes_{}_layers_{}",
                    "_factory_1_transforms_{}"
                ),
                CONFIGURATION_TAG,
                self.base.routes,
                self.base.middleware_layers,
                self.plan().route_transforms()
            ),
        }
    }

    /// Reject workloads whose accounting cannot be asserted.
    pub(super) fn validate(self) -> Result<(), String> {
        // `StartupFixturePlan` floors both counts at one, so a zero here would
        // silently measure a different application than the label claims.
        if self.base.routes == 0 || self.base.middleware_layers == 0 {
            return Err(format!(
                "routes ({}) and middleware_layers ({}) must both be at least 1",
                self.base.routes, self.base.middleware_layers
            ));
        }
        match (self.phase, self.base.path) {
            (ConfiguredPhase::PreparedThenServe, _) => self.base.validate(),
            (ConfiguredPhase::PreparationOnly, StartupPath::PreparedRoot) => Ok(()),
            (ConfiguredPhase::PreparationOnly, path) => Err(format!(
                "preparation-only requires the prepared-root topology, not {}",
                path.label()
            )),
        }
    }
}

/// Build the configured application for one instrumentation.
///
/// The counted factory records the invocation and every middleware
/// transformation, so the configured workload's counts stay directly comparable
/// with [`super::measure_startup`]'s. Exposed so a test can assert that the
/// prepared application really installs each component the label names, rather
/// than trusting the label alone.
///
/// # Errors
///
/// Returns a description of a factory failure or of a lifecycle registration
/// rejection.
pub fn build_configured_app(
    instrumentation: &ConnectionStartupInstrumentation,
    workload: ConfiguredStartupWorkload,
) -> Result<ConfiguredStartupTestApp, String> {
    let factory = counted_app_factory(instrumentation.clone(), workload.plan());
    let app = factory().map_err(|error| format!("factory failed: {error}"))?;
    app.with_message_assembler(TestAssembler)
        .with_protocol(ConfiguredProtocol)
        .on_connection_setup(|| async { ConfiguredConnectionState { id: 1 } })
        .map_err(|error| format!("setup registration failed: {error}"))?
        .on_connection_teardown(|state: ConfiguredConnectionState| async move { drop(state) })
        .map_err(|error| format!("teardown registration failed: {error}"))
}

/// Serve one connection with an immediate clean end-of-stream.
async fn serve_connection(
    prepared: &wireframe::app::PreparedApp<BincodeSerializer, ConfiguredConnectionState, Envelope>,
) -> Result<(), String> {
    let (client, server) = tokio::io::duplex(64);
    // Dropping the client half first turns the connection into an immediate
    // clean end-of-stream, so no frame handling is timed here.
    drop(client);
    prepared
        .handle_connection_result(server)
        .await
        .map_err(|error| format!("configured connection failed: {error}"))
}

/// Prepare once, serve every connection, then drop the root.
async fn prepare_and_serve(
    workload: ConfiguredStartupWorkload,
    instrumentation: &ConnectionStartupInstrumentation,
) -> Result<(), String> {
    let app = build_configured_app(instrumentation, workload)?;
    let prepared = app
        .prepare()
        .await
        .map_err(|error| format!("preparation failed: {error}"))?;
    for _ in 0..workload.base.connections {
        serve_connection(&prepared).await?;
    }
    drop(prepared);
    Ok(())
}

/// Prepare once and drop the root without serving a connection.
async fn prepare_only(
    workload: ConfiguredStartupWorkload,
    instrumentation: &ConnectionStartupInstrumentation,
) -> Result<(), String> {
    let app = build_configured_app(instrumentation, workload)?;
    let prepared = app
        .prepare()
        .await
        .map_err(|error| format!("preparation failed: {error}"))?;
    drop(prepared);
    Ok(())
}

/// Run one iteration of the configured workload against fresh counters.
async fn configured_once(
    workload: ConfiguredStartupWorkload,
    instrumentation: &ConnectionStartupInstrumentation,
) -> Result<(), String> {
    match workload.phase {
        ConfiguredPhase::PreparedThenServe => prepare_and_serve(workload, instrumentation).await,
        ConfiguredPhase::PreparationOnly => prepare_only(workload, instrumentation).await,
    }
}

/// Measure the configured workload, including each iteration's root drop.
///
/// # Errors
///
/// Returns a description of the invalid workload, factory failure, preparation
/// failure, or connection failure.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::runtime_ownership_benchmarks::{
///     ConfiguredPhase,
///     ConfiguredStartupWorkload,
///     StartupPath,
///     StartupWorkload,
///     measure_configured_startup,
/// };
///
/// # async fn example() -> Result<(), String> {
/// let measurement = measure_configured_startup(
///     ConfiguredStartupWorkload {
///         base: StartupWorkload {
///             path: StartupPath::PreparedRoot,
///             routes: 2,
///             middleware_layers: 2,
///             connections: 4,
///         },
///         phase: ConfiguredPhase::PreparationOnly,
///     },
///     1,
/// )
/// .await?;
/// assert_eq!(measurement.factory_calls, 1);
/// assert_eq!(measurement.transforms, 4);
/// # Ok(())
/// # }
/// ```
pub async fn measure_configured_startup(
    workload: ConfiguredStartupWorkload,
    iterations: u64,
) -> Result<StartupMeasurement, String> {
    workload.validate()?;
    if iterations == 0 {
        return Ok(StartupMeasurement {
            operations: 0,
            connections: workload.served_connections(),
            factory_calls: 0,
            transforms: 0,
            elapsed: std::time::Duration::ZERO,
        });
    }

    // The counters belong to this measurement, not to any earlier warm-up, so
    // they are created inside the timed region.
    let started = std::time::Instant::now();
    let mut counts = ConnectionStartupCounts::default();
    for _ in 0..iterations {
        let instrumentation = ConnectionStartupInstrumentation::new();
        configured_once(workload, &instrumentation).await?;
        counts = instrumentation.snapshot();
    }
    Ok(StartupMeasurement {
        operations: iterations,
        connections: workload.served_connections(),
        factory_calls: counts.factory_calls,
        transforms: counts.transforms,
        elapsed: started.elapsed(),
    })
}
