//! Unit coverage for the connection-startup fixture's own accounting.

use super::{ConnectionStartupInstrumentation, StartupFixturePlan, build_counted_app};

#[test]
fn plan_floors_zero_counts_at_one() {
    let plan = StartupFixturePlan::new(0, 0);
    assert_eq!(plan.routes(), 1, "plan.routes() should equal 1");
    assert_eq!(
        plan.middleware_layers(),
        1,
        "plan.middleware_layers() should equal 1"
    );
    assert_eq!(
        plan.route_transforms(),
        1,
        "plan.route_transforms() should equal 1"
    );
}

#[test]
fn plan_route_ids_are_one_based_and_contiguous() {
    assert_eq!(
        StartupFixturePlan::new(1, 1).route_ids(),
        vec![1],
        "StartupFixturePlan::new(1, 1).route_ids() should equal vec![1]"
    );
    assert_eq!(
        StartupFixturePlan::new(4, 1).route_ids(),
        vec![1, 2, 3, 4],
        "StartupFixturePlan::new(4, 1).route_ids() should equal vec![1, 2, 3, 4]"
    );
}

/// The fixture's two error types (`WireframeError`, `PrepareError`) do not
/// unify, and `panic_in_result_fn` rejects assertions in a `Result`-returning
/// test, so this returns `()` and surfaces setup failures with `expect`.
#[tokio::test]
async fn preparation_transforms_each_route_once_per_layer() {
    let instrumentation = ConnectionStartupInstrumentation::new();
    let plan = StartupFixturePlan::new(3, 2);
    let app = build_counted_app(&instrumentation, plan).expect("fixture app must build");
    assert_eq!(
        instrumentation.snapshot().transforms,
        0,
        "instrumentation.snapshot().transforms should equal 0"
    );

    let prepared = app.prepare().await.expect("preparation must succeed");
    assert_eq!(
        instrumentation.snapshot().transforms,
        plan.route_transforms(),
        "instrumentation.snapshot().transforms should equal plan.route_transforms()"
    );
    drop(prepared);
}
