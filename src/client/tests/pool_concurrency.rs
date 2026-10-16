//! Regression coverage for response ordering across pooled leases.

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

/// Keep asynchronous test steps bounded while still allowing socket teardown.
const GRACE: Duration = Duration::from_secs(1);

#[rstest]
#[tokio::test]
async fn pending_response_blocks_another_lease_from_using_the_socket() -> TestResult {
    let server = PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?;
    let pool = build_pooled_client(
        server.addr,
        ClientPoolConfig::default()
            .pool_size(1)
            .max_in_flight_per_socket(2),
        Arc::new(AtomicUsize::new(0)),
    )
    .await?;

    let first_lease = pool.acquire().await?;
    first_lease.send(&Ping(1)).await?;
    server.wait_for_delayed_first_response().await;

    let second_lease = pool.acquire().await?;
    let (second_call_started, second_call_started_rx) = tokio::sync::oneshot::channel();
    let mut second_call = tokio::spawn(async move {
        let _ = second_call_started.send(());
        second_lease.call::<Ping, Pong>(&Ping(2)).await
    });
    second_call_started_rx.await?;

    server.release_delayed_first_response();
    server.wait_for_delayed_first_response_attempt().await;
    assert!(
        timeout(Duration::from_millis(50), &mut second_call)
            .await
            .is_err(),
        "another lease used a socket with an unread split-phase response"
    );

    let first_response: Pong = first_lease.receive().await?;
    assert_eq!(first_response, Pong(1));

    let second_response: Pong = timeout(GRACE, &mut second_call).await???;
    assert_eq!(second_response, Pong(2));
    assert_eq!(server.connection_count(), 1);

    drop(first_lease);
    let released_lease = timeout(GRACE, pool.acquire()).await??;
    drop(released_lease);

    Ok(())
}
