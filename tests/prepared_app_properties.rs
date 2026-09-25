//! Property coverage for one-time prepared-application reuse.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use common::prepared_app::{
    TestApp,
    TransformCountingMiddleware,
    build_frame,
    handler,
    response_payload,
};
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use tokio::runtime::Builder;
use wireframe_testing::{TestResult, drive_prepared_with_frames};

// Generate bounded prepared-application cases and preserve one-time transforms.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prepared_app_transforms_once_and_reuses_services(
        route_count in 1usize..=4,
        middleware_layers in 0usize..=4,
        connection_count in 1usize..=4,
    ) {
        run_prepared_app_property_case(route_count, middleware_layers, connection_count)?;
    }
}

/// Exercise a generated preparation case on a deterministic Tokio runtime.
fn run_prepared_app_property_case(
    route_count: usize,
    middleware_layers: usize,
    connection_count: usize,
) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime
        .block_on(exercise_prepared_app_property_case(
            route_count,
            middleware_layers,
            connection_count,
        ))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Prepare a bounded generated application and verify every requested dispatch.
async fn exercise_prepared_app_property_case(
    route_count: usize,
    middleware_layers: usize,
    connection_count: usize,
) -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let tags = middleware_tags(middleware_layers)?;
    let mut app = TestApp::new()?;
    for route_id in 1..=route_count {
        app = app.route(u32::try_from(route_id)?, handler())?;
    }
    for tag in &tags {
        app = app.wrap(TransformCountingMiddleware {
            tag: *tag,
            transforms: Arc::clone(&transforms),
        })?;
    }

    let prepared = app
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let expected_transforms = route_count * middleware_layers;
    if transforms.load(Ordering::SeqCst) != expected_transforms {
        return Err(format!(
            "preparation transformed {} route services, expected {expected_transforms}",
            transforms.load(Ordering::SeqCst)
        )
        .into());
    }

    for route_id in (1..=route_count)
        .cycle()
        .take(route_count + connection_count)
    {
        let route_id = u32::try_from(route_id)?;
        let payload = vec![u8::try_from(route_id)?];
        let response =
            drive_prepared_with_frames(&prepared, vec![build_frame(route_id, payload)?]).await?;
        let mut expected = vec![u8::try_from(route_id)?];
        expected.extend(tags.iter().copied());
        expected.extend(tags.iter().rev().copied());
        if response_payload(&response)? != expected {
            return Err(
                format!("route {route_id} did not preserve generated middleware order").into(),
            );
        }
    }
    if transforms.load(Ordering::SeqCst) != expected_transforms {
        return Err(format!(
            "prepared connections rebuilt middleware: observed {}, expected {expected_transforms}",
            transforms.load(Ordering::SeqCst)
        )
        .into());
    }
    Ok(())
}

/// Build distinct middleware tags for a bounded generated layer count.
fn middleware_tags(layer_count: usize) -> TestResult<Vec<u8>> {
    (0..layer_count)
        .map(|layer| Ok(b'A' + u8::try_from(layer)?))
        .collect()
}
