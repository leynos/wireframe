//! Workloads and measurement for application connection startup.
//!
//! The prepared-connection path exists so an application's route and middleware
//! chains are evaluated once per server run rather than once per connection.
//! This module measures the two topologies the issue names, and — crucially —
//! reports the **factory invocation and middleware transformation counts** they
//! produce. Those counts are the whole point of the prepared path; a timing
//! difference alone cannot distinguish "prepared once" from "prepared per
//! connection", so the counts are recorded alongside the elapsed time and are
//! asserted exactly by the companion test suite.
//!
//! Route and middleware counts are workload parameters, so a run with a
//! different application shape needs no code change.
//!
//! # Paths
//!
//! [`StartupPath::PerConnectionFactory`] builds and prepares a fresh
//! application for every connection: `connections` factory calls and
//! `connections × routes × middleware_layers` transformations.
//!
//! [`StartupPath::PreparedRoot`] builds and prepares once, then serves every
//! connection from that single prepared root: `1` factory call and
//! `routes × middleware_layers` transformations, however many connections
//! follow.
//!
//! # Transport
//!
//! A connection is a `tokio::io::duplex` pair, not a socket. The issue permits
//! transport I/O only for this scenario, and the duplex pair keeps the measured
//! cost in application preparation and per-connection setup rather than in the
//! kernel loopback path. The client half is dropped before the connection is
//! served, so the server sees a clean end-of-stream and no frame round-trips are
//! measured.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use tokio::io::duplex;
use wireframe::test_helpers::connection_startup::{
    ConnectionStartupCounts,
    ConnectionStartupInstrumentation,
    StartupFixturePlan,
    StartupTestApp,
    counted_app_factory,
};

/// Capacity of each in-memory duplex pipe, in bytes.
const DUPLEX_CAPACITY: usize = 64;

/// Route count used by the benchmark's default startup workload.
pub const DEFAULT_ROUTES: usize = 2;

/// Middleware layer count used by the benchmark's default startup workload.
pub const DEFAULT_MIDDLEWARE_LAYERS: usize = 2;

/// Connection count used by the benchmark's default startup workload.
pub const DEFAULT_CONNECTIONS: usize = 4;

/// Which startup topology a workload exercises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupPath {
    /// Build and prepare a fresh application for every connection.
    PerConnectionFactory,
    /// Build and prepare once and share the prepared root.
    PreparedRoot,
}

impl StartupPath {
    /// Short identifier used in benchmark labels.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::StartupPath;
    ///
    /// assert_eq!(StartupPath::PreparedRoot.label(), "prepared_root");
    /// ```
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PerConnectionFactory => "per_connection_factory",
            Self::PreparedRoot => "prepared_root",
        }
    }

    /// Factory invocations this path must record for a workload.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::StartupPath;
    ///
    /// assert_eq!(
    ///     StartupPath::PerConnectionFactory.expected_factory_calls(4),
    ///     4
    /// );
    /// assert_eq!(StartupPath::PreparedRoot.expected_factory_calls(4), 1);
    /// ```
    #[must_use]
    pub const fn expected_factory_calls(self, connections: usize) -> usize {
        match self {
            Self::PerConnectionFactory => connections,
            Self::PreparedRoot => 1,
        }
    }

    /// Middleware transformations this path must record for a workload.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::StartupPath;
    ///
    /// assert_eq!(StartupPath::PreparedRoot.expected_transforms(4, 3, 2), 6);
    /// assert_eq!(
    ///     StartupPath::PerConnectionFactory.expected_transforms(4, 3, 2),
    ///     24
    /// );
    /// ```
    #[must_use]
    pub const fn expected_transforms(
        self,
        connections: usize,
        routes: usize,
        layers: usize,
    ) -> usize {
        let per_preparation = routes * layers;
        match self {
            Self::PerConnectionFactory => connections * per_preparation,
            Self::PreparedRoot => per_preparation,
        }
    }
}

/// One connection-startup scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupWorkload {
    /// Startup topology to exercise.
    pub path: StartupPath,
    /// Routes registered on the fixture application.
    pub routes: usize,
    /// Middleware layers wrapped around every route.
    pub middleware_layers: usize,
    /// Connections served per iteration.
    pub connections: usize,
}

impl Default for StartupWorkload {
    fn default() -> Self {
        Self {
            path: StartupPath::PreparedRoot,
            routes: DEFAULT_ROUTES,
            middleware_layers: DEFAULT_MIDDLEWARE_LAYERS,
            connections: DEFAULT_CONNECTIONS,
        }
    }
}

impl StartupWorkload {
    /// The fixture plan this workload builds.
    #[must_use]
    pub const fn plan(self) -> StartupFixturePlan {
        StartupFixturePlan::new(self.routes, self.middleware_layers)
    }

    /// Benchmark label embedding the parameters and both exact counts.
    ///
    /// The counts are in the label so a baseline table can be read without
    /// cross-referencing the test suite: a label that claims `prepared_root`
    /// while showing per-connection counts would be visibly wrong.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{StartupPath, StartupWorkload};
    ///
    /// let workload = StartupWorkload {
    ///     path: StartupPath::PreparedRoot,
    ///     routes: 2,
    ///     middleware_layers: 2,
    ///     connections: 4,
    /// };
    /// assert_eq!(
    ///     workload.label(),
    ///     "prepared_root_routes_2_layers_2_connections_4_factory_1_transforms_4"
    /// );
    /// ```
    #[must_use]
    pub fn label(self) -> String {
        format!(
            "{}_routes_{}_layers_{}_connections_{}_factory_{}_transforms_{}",
            self.path.label(),
            self.routes,
            self.middleware_layers,
            self.connections,
            self.path.expected_factory_calls(self.connections),
            self.path
                .expected_transforms(self.connections, self.routes, self.middleware_layers)
        )
    }

    /// Reject workloads whose accounting cannot be asserted.
    fn validate(self) -> Result<(), String> {
        if self.connections == 0 {
            return Err("connections must be at least 1".to_string());
        }
        // `StartupFixturePlan::new` floors both counts at one, so a zero here
        // would silently measure a different application than the label claims.
        if self.routes == 0 || self.middleware_layers == 0 {
            return Err(format!(
                "routes ({}) and middleware_layers ({}) must both be at least 1",
                self.routes, self.middleware_layers
            ));
        }
        Ok(())
    }
}

/// Result of measuring one startup workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupMeasurement {
    /// Workload iterations completed.
    pub operations: u64,
    /// Connections served per iteration.
    pub connections: usize,
    /// Factory invocations recorded in the final iteration.
    pub factory_calls: usize,
    /// Middleware transformations recorded in the final iteration.
    pub transforms: usize,
    /// Wall-clock time across every iteration.
    pub elapsed: Duration,
}

/// Serve one connection from an already-prepared application.
async fn serve_connection(prepared: &wireframe::app::PreparedApp) -> Result<(), String> {
    let (client, server) = duplex(DUPLEX_CAPACITY);
    // Dropping the client half before serving turns the connection into a clean
    // immediate end-of-stream, so no frame handling is timed here.
    drop(client);
    prepared
        .handle_connection_result(server)
        .await
        .map_err(|err| format!("prepared connection failed: {err}"))
}

/// Build and prepare a fresh application, serve one connection, then discard it.
async fn serve_via_per_connection_factory(
    instrumentation: &ConnectionStartupInstrumentation,
    workload: StartupWorkload,
) -> Result<(), String> {
    let factory = counted_app_factory(instrumentation.clone(), workload.plan());
    let app: StartupTestApp = factory().map_err(|err| format!("factory failed: {err}"))?;
    let prepared = app
        .prepare()
        .await
        .map_err(|err| format!("preparation failed: {err}"))?;
    serve_connection(&prepared).await
}

/// Prepare once and serve every connection from the shared root.
async fn serve_via_prepared_root(
    instrumentation: &ConnectionStartupInstrumentation,
    workload: StartupWorkload,
) -> Result<(), String> {
    let factory = counted_app_factory(instrumentation.clone(), workload.plan());
    let app: StartupTestApp = factory().map_err(|err| format!("factory failed: {err}"))?;
    let prepared = app
        .prepare()
        .await
        .map_err(|err| format!("preparation failed: {err}"))?;
    for _ in 0..workload.connections {
        serve_connection(&prepared).await?;
    }
    Ok(())
}

/// Run one iteration and report the counters it recorded.
async fn startup_once(
    workload: StartupWorkload,
    instrumentation: &ConnectionStartupInstrumentation,
) -> Result<ConnectionStartupCounts, String> {
    match workload.path {
        StartupPath::PerConnectionFactory => {
            for _ in 0..workload.connections {
                serve_via_per_connection_factory(instrumentation, workload).await?;
            }
        }
        StartupPath::PreparedRoot => {
            serve_via_prepared_root(instrumentation, workload).await?;
        }
    }
    Ok(black_box(instrumentation.snapshot()))
}

/// Attempt `iterations` startup workloads, reporting the last iteration's counts.
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
///     StartupPath,
///     StartupWorkload,
///     measure_startup,
/// };
///
/// # async fn example() -> Result<(), String> {
/// let measurement = measure_startup(
///     StartupWorkload {
///         path: StartupPath::PreparedRoot,
///         routes: 2,
///         middleware_layers: 2,
///         connections: 4,
///     },
///     1,
/// )
/// .await?;
/// assert_eq!(measurement.factory_calls, 1);
/// assert_eq!(measurement.transforms, 4);
/// # Ok(())
/// # }
/// ```
pub async fn measure_startup(
    workload: StartupWorkload,
    iterations: u64,
) -> Result<StartupMeasurement, String> {
    workload.validate()?;
    if iterations == 0 {
        return Ok(StartupMeasurement {
            operations: 0,
            connections: workload.connections,
            factory_calls: 0,
            transforms: 0,
            elapsed: Duration::ZERO,
        });
    }

    // Counters are created inside the timed region so the recorded figures
    // describe this measurement and not any earlier warm-up.
    let started = Instant::now();
    let mut counts = ConnectionStartupCounts::default();
    for _ in 0..iterations {
        let instrumentation = ConnectionStartupInstrumentation::new();
        counts = startup_once(workload, &instrumentation).await?;
    }
    let elapsed = started.elapsed();

    Ok(StartupMeasurement {
        operations: iterations,
        connections: workload.connections,
        factory_calls: counts.factory_calls,
        transforms: counts.transforms,
        elapsed,
    })
}

#[cfg(test)]
mod tests {
    //! Unit coverage for startup accounting and workload rejection.

    use super::{StartupPath, StartupWorkload, measure_startup};

    #[test]
    fn counts_scale_with_the_workload_shape() {
        assert_eq!(StartupPath::PreparedRoot.expected_factory_calls(16), 1);
        assert_eq!(
            StartupPath::PerConnectionFactory.expected_factory_calls(16),
            16
        );
        assert_eq!(StartupPath::PreparedRoot.expected_transforms(16, 4, 3), 12);
        assert_eq!(
            StartupPath::PerConnectionFactory.expected_transforms(16, 4, 3),
            192
        );
    }

    #[tokio::test]
    async fn zero_connections_is_rejected() {
        let error = measure_startup(
            StartupWorkload {
                connections: 0,
                ..StartupWorkload::default()
            },
            1,
        )
        .await
        .expect_err("a workload with no connections must be rejected");

        assert!(
            error.contains("connections must be at least 1"),
            "unexpected: {error}"
        );
    }

    #[tokio::test]
    async fn prepared_root_transforms_each_route_once_per_layer() {
        let measurement = measure_startup(
            StartupWorkload {
                path: StartupPath::PreparedRoot,
                routes: 3,
                middleware_layers: 2,
                connections: 4,
            },
            1,
        )
        .await
        .expect("a valid startup workload must measure");

        assert_eq!(measurement.factory_calls, 1);
        assert_eq!(measurement.transforms, 6);
    }
}
