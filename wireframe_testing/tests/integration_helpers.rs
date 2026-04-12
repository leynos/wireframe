//! Integration coverage for `wireframe_testing` integration helpers.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use wireframe_testing::{
    CommonTestEnvelope,
    TestResult,
    echo_app_factory,
    spawn_wireframe_pair_default,
};

const ROUTE_ID: u32 = 1;

#[tokio::test]
async fn integration_helpers_round_trip() -> TestResult {
    let counter = Arc::new(AtomicUsize::new(0));
    let factory = echo_app_factory(&counter);
    let mut pair = spawn_wireframe_pair_default(factory).await?;
    let request = CommonTestEnvelope::new(ROUTE_ID, Some(7), vec![1, 2, 3]);
    let response: CommonTestEnvelope = pair.client_mut()?.call(&request).await?;

    if response.id != ROUTE_ID {
        return Err(format!("unexpected response id: {}", response.id).into());
    }
    if response.payload.as_slice() != [1, 2, 3] {
        return Err("response payload mismatch".into());
    }
    if response.correlation_id != Some(7) {
        return Err(format!(
            "expected correlation id Some(7), got {:?}",
            response.correlation_id
        )
        .into());
    }
    if counter.load(Ordering::SeqCst) != 1 {
        return Err("expected handler to be invoked exactly once".into());
    }

    pair.shutdown().await
}
