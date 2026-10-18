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
