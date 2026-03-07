//! Unit tests for pooled wireframe clients.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rstest::{fixture, rstest};
use tokio::time::{advance, timeout};

use crate::{
    client::{ClientError, ClientPoolConfig},
    test_helpers::{
        Ping,
        Pong,
        PoolServerBehavior,
        PoolTestServer,
        TestClientPool,
        build_pooled_client,
    },
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[rustfmt::skip]
#[fixture]
fn client_pool_config() -> ClientPoolConfig {
    ClientPoolConfig::default()
}

async fn run_ping_round_trip(pool: &TestClientPool, ping: u8) -> Result<(), ClientError> {
    let lease = pool.acquire().await?;
    let reply: Pong = lease.call(&Ping(ping)).await?;
    assert_eq!(reply, Pong(ping));
    Ok(())
}

/// Scenario values for [`run_two_lease_scenario`].
struct TwoLeaseParams {
    first_ping: u8,
    second_ping: u8,
    expected_preamble_count: usize,
    expected_connection_count: usize,
}

async fn run_two_lease_scenario<F, Fut>(
    config: ClientPoolConfig,
    preamble_callback_count: Arc<AtomicUsize>,
    between_leases: F,
    params: TwoLeaseParams,
) -> TestResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let server = PoolTestServer::start().await?;
    let pool = build_pooled_client(server.addr, config, preamble_callback_count.clone()).await?;

    run_ping_round_trip(&pool, params.first_ping).await?;
    between_leases().await;
    run_ping_round_trip(&pool, params.second_ping).await?;
    assert_eq!(
        preamble_callback_count.load(Ordering::SeqCst),
        params.expected_preamble_count
    );
    assert_eq!(server.preamble_count(), params.expected_preamble_count);
    assert_eq!(server.connection_count(), params.expected_connection_count);

    Ok(())
}

#[rstest]
#[tokio::test]
async fn pooled_reuse_preserves_preamble_state(client_pool_config: ClientPoolConfig) -> TestResult {
    run_two_lease_scenario(
        client_pool_config.pool_size(1),
        Arc::new(AtomicUsize::new(0)),
        || async {},
        TwoLeaseParams {
            first_ping: 7,
            second_ping: 8,
            expected_preamble_count: 1,
            expected_connection_count: 1,
        },
    )
    .await
}

#[rstest]
#[tokio::test]
async fn pool_enforces_per_socket_in_flight_limit(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let server = PoolTestServer::start().await?;
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1).max_in_flight_per_socket(2),
        Arc::new(AtomicUsize::new(0)),
    )
    .await?;

    let first = pool.acquire().await?;
    let second = pool.acquire().await?;
    let third = timeout(Duration::from_millis(25), pool.acquire()).await;
    assert!(third.is_err(), "third acquire should wait for a permit");

    drop(first);
    let third = timeout(Duration::from_millis(100), pool.acquire()).await?;
    let _third = third?;
    drop(second);
    Ok(())
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn idle_timeout_recycles_socket_and_replays_preamble(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let idle_timeout = Duration::from_millis(50);
    run_two_lease_scenario(
        client_pool_config.pool_size(1).idle_timeout(idle_timeout),
        Arc::new(AtomicUsize::new(0)),
        || async move {
            advance(idle_timeout + idle_timeout).await;
            tokio::task::yield_now().await;
        },
        TwoLeaseParams {
            first_ping: 1,
            second_ping: 2,
            expected_preamble_count: 2,
            expected_connection_count: 2,
        },
    )
    .await
}

#[rstest]
#[tokio::test]
async fn protocol_error_marks_connection_broken_and_reconnects(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let server =
        PoolTestServer::start_with_behavior(PoolServerBehavior::MalformedFirstResponse).await?;
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1),
        preamble_callback_count.clone(),
    )
    .await?;

    let lease = pool.acquire().await?;
    let error = lease
        .call::<Ping, Pong>(&Ping(1))
        .await
        .expect_err("malformed response should fail decoding");
    assert!(error.should_recycle_connection());
    drop(lease);

    let lease = pool.acquire().await?;
    let reply: Pong = lease.call(&Ping(2)).await?;
    assert_eq!(reply, Pong(2));

    assert_eq!(preamble_callback_count.load(Ordering::SeqCst), 2);
    assert_eq!(server.preamble_count(), 2);
    assert_eq!(server.connection_count(), 2);

    Ok(())
}
