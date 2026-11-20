//! Unit coverage for startup accounting and workload rejection.

use wireframe::test_helpers::connection_startup::ConnectionStartupInstrumentation;

use super::{
    ConfiguredPhase,
    ConfiguredStartupWorkload,
    StartupPath,
    StartupWorkload,
    build_configured_app,
    measure_configured_startup,
    measure_startup,
};

#[test]
fn counts_scale_with_the_workload_shape() {
    assert_eq!(
        StartupPath::PreparedRoot.expected_factory_calls(16),
        1,
        "StartupPath::PreparedRoot.expected_factory_calls(16) should equal 1"
    );
    assert_eq!(
        StartupPath::PerConnectionFactory.expected_factory_calls(16),
        16,
        "StartupPath::PerConnectionFactory.expected_factory_calls(16) should equal 16"
    );
    assert_eq!(
        StartupPath::PreparedRoot.expected_transforms(16, 4, 3),
        12,
        "StartupPath::PreparedRoot.expected_transforms(16, 4, 3) should equal 12"
    );
    assert_eq!(
        StartupPath::PerConnectionFactory.expected_transforms(16, 4, 3),
        192,
        "StartupPath::PerConnectionFactory.expected_transforms(16, 4, 3) should equal 192"
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

    assert_eq!(
        measurement.factory_calls, 1,
        "measurement.factory_calls should equal 1"
    );
    assert_eq!(
        measurement.transforms, 6,
        "measurement.transforms should equal 6"
    );
}

/// A configured workload with the given phase and connection count.
fn configured(phase: ConfiguredPhase, connections: usize) -> ConfiguredStartupWorkload {
    ConfiguredStartupWorkload {
        base: StartupWorkload {
            path: StartupPath::PreparedRoot,
            routes: 3,
            middleware_layers: 2,
            connections,
        },
        phase,
    }
}

#[tokio::test]
async fn configured_label_names_every_component_and_the_base_workload() {
    let serving = configured(ConfiguredPhase::PreparedThenServe, 4);
    assert_eq!(
        serving.label(),
        concat!(
            "configured_serializer_1_assembler_1_callbacks_2_protocol_1_",
            "prepared_root_routes_3_layers_2_connections_4_",
            "factory_1_transforms_6",
        ),
        "the serving label should state the components and reuse the base label"
    );

    let preparing = configured(ConfiguredPhase::PreparationOnly, 0);
    assert_eq!(
        preparing.label(),
        concat!(
            "configured_serializer_1_assembler_1_callbacks_2_protocol_1_",
            "preparation_only_prepared_root_routes_3_layers_2_",
            "factory_1_transforms_6",
        ),
        "the preparation label must not claim a connection count"
    );
    assert_ne!(
        serving.label(),
        preparing.label(),
        "the two phases must not share a benchmark identifier"
    );
}

#[tokio::test]
async fn configured_application_installs_every_labelled_component() {
    let instrumentation = ConnectionStartupInstrumentation::new();
    let app = build_configured_app(
        &instrumentation,
        configured(ConfiguredPhase::PreparedThenServe, 1),
    )
    .expect("the configured application should build");
    let prepared = app
        .prepare()
        .await
        .expect("the configured application should prepare");

    assert!(
        prepared.message_assembler().is_some(),
        "the label claims an assembler, so the prepared root must own one"
    );
    assert!(
        prepared.protocol().is_some(),
        "the label claims a protocol, so the prepared root must own one"
    );
}

#[tokio::test]
async fn configured_serving_prepares_once_for_every_connection() {
    // The default base path is the prepared root, so one factory call serves
    // all four connections.
    let measurement =
        measure_configured_startup(configured(ConfiguredPhase::PreparedThenServe, 4), 1)
            .await
            .expect("a valid configured workload must measure");

    assert_eq!(measurement.operations, 1, "operations should equal 1");
    assert_eq!(
        measurement.connections, 4,
        "the serving phase should report the connections it served"
    );
    assert_eq!(
        measurement.factory_calls, 1,
        "the prepared root should build the application once"
    );
    assert_eq!(
        measurement.transforms, 6,
        "one preparation should transform three routes through two layers"
    );
}

#[tokio::test]
async fn configured_per_connection_factory_builds_once_per_connection() {
    let mut workload = configured(ConfiguredPhase::PreparedThenServe, 4);
    workload.base.path = StartupPath::PerConnectionFactory;

    assert_eq!(
        workload.label(),
        concat!(
            "configured_serializer_1_assembler_1_callbacks_2_protocol_1_",
            "per_connection_factory_routes_3_layers_2_connections_4_",
            "factory_4_transforms_24",
        ),
        "the label should state the counts the measurement must record"
    );

    let measurement = measure_configured_startup(workload, 1)
        .await
        .expect("a valid configured workload must measure");

    assert_eq!(
        measurement.connections, 4,
        "the serving phase should report the connections it served"
    );
    assert_eq!(
        measurement.factory_calls, 4,
        "the per-connection topology should build once per connection"
    );
    assert_eq!(
        measurement.transforms, 24,
        "four preparations should each transform three routes through two layers"
    );
}

#[tokio::test]
async fn configured_preparation_serves_nothing_and_keeps_the_counts() {
    let measurement =
        measure_configured_startup(configured(ConfiguredPhase::PreparationOnly, 0), 1)
            .await
            .expect("a valid preparation workload must measure");

    assert_eq!(
        measurement.connections, 0,
        "preparation-only serves no connection"
    );
    assert_eq!(
        measurement.factory_calls, 1,
        "preparation-only builds the application once"
    );
    assert_eq!(
        measurement.transforms, 6,
        "preparation-only still transforms three routes through two layers"
    );
}

#[tokio::test]
async fn configured_zero_iterations_report_zero_counts() {
    let measurement =
        measure_configured_startup(configured(ConfiguredPhase::PreparedThenServe, 4), 0)
            .await
            .expect("zero iterations is a valid measurement");

    assert_eq!(measurement.operations, 0, "operations should equal 0");
    assert_eq!(measurement.factory_calls, 0, "factory_calls should equal 0");
    assert_eq!(measurement.transforms, 0, "transforms should equal 0");
    assert_eq!(
        measurement.elapsed,
        std::time::Duration::ZERO,
        "an unrun measurement should report no elapsed time"
    );
}

#[tokio::test]
async fn configured_preparation_rejects_the_per_connection_topology() {
    let mut workload = configured(ConfiguredPhase::PreparationOnly, 0);
    workload.base.path = StartupPath::PerConnectionFactory;
    let error = measure_configured_startup(workload, 1)
        .await
        .expect_err("preparation-only is meaningless per connection");

    assert!(
        error.contains("preparation-only requires the prepared-root topology"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn configured_zero_routes_is_rejected() {
    let mut workload = configured(ConfiguredPhase::PreparedThenServe, 1);
    workload.base.routes = 0;
    let error = measure_configured_startup(workload, 1)
        .await
        .expect_err("a route-less workload would measure a different application");

    assert!(
        error.contains("must both be at least 1"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn configured_zero_connections_is_rejected_when_serving() {
    let error = measure_configured_startup(configured(ConfiguredPhase::PreparedThenServe, 0), 1)
        .await
        .expect_err("a serving workload with no connections must be rejected");

    assert!(
        error.contains("connections must be at least 1"),
        "unexpected: {error}"
    );
}
