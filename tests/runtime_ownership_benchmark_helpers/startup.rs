//! V-O2 — factory and middleware counts are exact, not inferred.
//!
//! Both startup paths make a different claim about how often the application
//! factory runs and how often middleware is applied. Neither is observable in a
//! timing, so both are asserted as exact counts over a small truth table of
//! route and layer combinations.

use rstest::rstest;
use wireframe_testing::runtime_ownership_benchmarks::{
    StartupPath,
    StartupWorkload,
    measure_startup,
};

use super::harness::{ITERATIONS, block_on};

#[rstest]
#[case::one_route_one_layer(1, 1, 1)]
#[case::two_routes_two_layers(2, 2, 4)]
#[case::four_routes_three_layers(4, 3, 12)]
fn prepared_root_transforms_once_regardless_of_connection_count(
    #[case] routes: usize,
    #[case] middleware_layers: usize,
    #[case] connections: usize,
) {
    let measurement = block_on(measure_startup(
        StartupWorkload {
            path: StartupPath::PreparedRoot,
            routes,
            middleware_layers,
            connections,
        },
        ITERATIONS,
    ))
    .expect("prepared-root startup measurement failed");

    assert_eq!(
        measurement.factory_calls, 1,
        "a shared prepared root is built once for all {connections} connections"
    );
    assert_eq!(
        measurement.transforms,
        routes * middleware_layers,
        "middleware is applied once per route per layer, not once per connection"
    );
}

#[rstest]
#[case::one_route_one_layer(1, 1, 1)]
#[case::two_routes_two_layers(2, 2, 4)]
#[case::four_routes_three_layers(4, 3, 12)]
fn per_connection_factory_transforms_for_every_connection(
    #[case] routes: usize,
    #[case] middleware_layers: usize,
    #[case] connections: usize,
) {
    let measurement = block_on(measure_startup(
        StartupWorkload {
            path: StartupPath::PerConnectionFactory,
            routes,
            middleware_layers,
            connections,
        },
        ITERATIONS,
    ))
    .expect("per-connection startup measurement failed");

    assert_eq!(
        measurement.factory_calls, connections,
        "the per-connection path builds a fresh application for each connection"
    );
    assert_eq!(
        measurement.transforms,
        connections * routes * middleware_layers,
        "every connection re-applies the whole middleware chain"
    );
}

#[test]
fn startup_counts_appear_in_the_benchmark_label() {
    let workload = StartupWorkload {
        path: StartupPath::PreparedRoot,
        routes: 2,
        middleware_layers: 2,
        connections: 4,
    };
    let label = workload.label();
    assert!(
        label.contains("factory_1") && label.contains("transforms_4"),
        "the label must carry the exact counts a baseline table quotes: {label}"
    );
}
