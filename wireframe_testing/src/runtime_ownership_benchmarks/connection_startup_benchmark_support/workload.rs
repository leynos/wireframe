//! The connection-startup scenario as data.
//!
//! Everything here describes a workload rather than performing one: the
//! parameters a baseline must state, the exact factory and transformation
//! counts each path must produce, and the result a measurement reports. The
//! counts are derived from the workload's own shape, so a caller can assert
//! them without running anything, and a run with a different application shape
//! needs no code change.

use std::time::Duration;

use wireframe::test_helpers::connection_startup::StartupFixturePlan;

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
    pub(super) fn validate(self) -> Result<(), String> {
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
