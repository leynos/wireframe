//! Integration coverage for `wireframe_testing` integration helpers.

use std::sync::Arc;

use futures::future::BoxFuture;
use tokio::sync::oneshot;
use wireframe::{WireframeClient, app::Envelope, server::WireframeServer};
use wireframe_testing::{CommonTestEnvelope, TestResult, factory, unused_listener};

const ROUTE_ID: u32 = 7;

#[tokio::test]
#[expect(
    clippy::expect_used,
    reason = "route registration should fail loudly in integration helper tests"
)]
async fn integration_helpers_round_trip() -> TestResult {
    let base_factory = factory();
    let handler = Arc::new(|_env: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) });

    let server = WireframeServer::new(move || {
        let app = base_factory();
        app.route(ROUTE_ID, handler.clone())
            .expect("route registration should succeed")
    })
    .workers(1);

    let listener = unused_listener()?;
    let server = server.bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server missing local addr")?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    let mut client = WireframeClient::builder().connect(addr).await?;

    let request = CommonTestEnvelope::with_payload(ROUTE_ID, vec![1, 2, 3]);
    let response: CommonTestEnvelope = client.call_correlated(request).await?;

    if response.id != ROUTE_ID {
        return Err(format!("unexpected response id: {}", response.id).into());
    }
    if response.payload != vec![1, 2, 3] {
        return Err("response payload mismatch".into());
    }
    if response.correlation_id.is_none() {
        return Err("expected correlation id to be set".into());
    }

    let _ = shutdown_tx.send(());
    handle.await??;

    Ok(())
}
