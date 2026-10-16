//! Cancellation regression tests for pooled-client lease dispatch.

use std::{
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use rstest::rstest;
use tokio::time::timeout;

use crate::{
    client::ClientPoolConfig,
    test_helpers::{Ping, Pong, PoolServerBehavior, PoolTestServer, build_pooled_client},
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Upper bound on asynchronous test steps and teardown.
const GRACE: Duration = Duration::from_secs(1);

#[rstest]
#[tokio::test]
async fn checkout_cancellation_keeps_pending_protocol_reservation() -> TestResult {
    let server = PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?;
    let pool = build_pooled_client(
        server.addr,
        ClientPoolConfig::default()
            .pool_size(1)
            .max_in_flight_per_socket(2),
        Arc::new(AtomicUsize::new(0)),
    )
    .await?;

    let lease = pool.acquire().await?;
    lease.send(&Ping(1)).await?;
    timeout(GRACE, server.wait_for_delayed_first_response()).await?;

    let checked_out = lease.slot.pool.get().await?;
    assert!(
        timeout(Duration::from_millis(50), lease.receive::<Pong>())
            .await
            .is_err()
    );
    assert!(
        lease.pending_response_permit.lock().await.is_some(),
        "cancelling checkout must keep the pending-response reservation"
    );

    drop(checked_out);
    server.release_delayed_first_response();
    timeout(GRACE, server.wait_for_delayed_first_response_attempt()).await?;

    let response: Pong = timeout(GRACE, lease.receive()).await??;
    assert_eq!(response, Pong(1));
    assert!(lease.pending_response_permit.lock().await.is_none());

    Ok(())
}
