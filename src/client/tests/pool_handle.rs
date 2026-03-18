//! Unit tests for the `PoolHandle` fairness API.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rstest::{fixture, rstest};
use tokio::{
    sync::Mutex,
    time::{advance, timeout},
};

use crate::{
    client::{ClientError, ClientPoolConfig, PoolFairnessPolicy},
    test_helpers::{
        Ping,
        Pong,
        PoolTestServer,
        TestClientPool,
        acquire_and_record,
        build_pooled_client,
        build_preamble_pool,
    },
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[rustfmt::skip]
#[fixture]
fn client_pool_config() -> ClientPoolConfig {
    ClientPoolConfig::default()
}

async fn build_handle_pool(
    config: ClientPoolConfig,
) -> Result<(PoolTestServer, TestClientPool), ClientError> {
    let server = PoolTestServer::start().await?;
    let pool = build_pooled_client(server.addr, config, Arc::new(AtomicUsize::new(0))).await?;
    Ok((server, pool))
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn round_robin_handles_share_one_socket_fairly(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let (_server, pool) = build_handle_pool(
        client_pool_config
            .pool_size(1)
            .max_in_flight_per_socket(1)
            .fairness_policy(PoolFairnessPolicy::RoundRobin),
    )
    .await?;
    let grants = Arc::new(Mutex::new(Vec::new()));
    let first = pool.handle();
    let second = pool.handle();

    let left = tokio::spawn(acquire_and_record(first, "a", 3, Arc::clone(&grants)));
    let right = tokio::spawn(acquire_and_record(second, "b", 3, Arc::clone(&grants)));
    let (left_result, right_result) = tokio::join!(left, right);
    left_result??;
    right_result??;

    let observed = grants.lock().await.clone();
    assert_eq!(observed.len(), 6, "expected 6 total grants");

    let a_count = observed.iter().filter(|&label| *label == "a").count();
    let b_count = observed.iter().filter(|&label| *label == "b").count();
    assert_eq!(a_count, 3, "expected 3 grants for session-a");
    assert_eq!(b_count, 3, "expected 3 grants for session-b");

    for i in 1..observed.len() {
        let prev = observed.get(i - 1).expect("valid index i-1");
        let curr = observed.get(i).expect("valid index i");
        assert_ne!(prev, curr, "expected strict alternation at position {i}");
    }
    Ok(())
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn fifo_policy_preserves_wait_order(client_pool_config: ClientPoolConfig) -> TestResult {
    let (_server, pool) = build_handle_pool(
        client_pool_config
            .pool_size(1)
            .max_in_flight_per_socket(1)
            .fairness_policy(PoolFairnessPolicy::Fifo),
    )
    .await?;
    let blocker = pool.acquire().await?;
    let grants = Arc::new(Mutex::new(Vec::new()));

    let first = tokio::spawn(acquire_and_record(
        pool.handle(),
        "first",
        1,
        Arc::clone(&grants),
    ));
    tokio::task::yield_now().await;
    let second = tokio::spawn(acquire_and_record(
        pool.handle(),
        "second",
        1,
        Arc::clone(&grants),
    ));
    tokio::task::yield_now().await;
    let third = tokio::spawn(acquire_and_record(
        pool.handle(),
        "third",
        1,
        Arc::clone(&grants),
    ));
    tokio::task::yield_now().await;

    drop(blocker);
    first.await??;
    second.await??;
    third.await??;

    let observed = grants.lock().await.clone();
    assert_eq!(observed, vec!["first", "second", "third"]);
    Ok(())
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn handle_acquire_respects_back_pressure(client_pool_config: ClientPoolConfig) -> TestResult {
    let (_server, pool) = build_handle_pool(client_pool_config.pool_size(1)).await?;
    let mut first = pool.handle();
    let mut second = pool.handle();

    let held_lease = first.acquire().await?;
    let blocked = timeout(Duration::from_millis(25), second.acquire()).await;
    assert!(blocked.is_err(), "second handle should stay blocked");

    drop(held_lease);
    let recovered = timeout(Duration::from_millis(100), second.acquire()).await?;
    let _recovered = recovered?;
    Ok(())
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn handle_acquire_dropped_waiter_does_not_leak_capacity(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let (_server, pool) = build_handle_pool(client_pool_config.pool_size(1)).await?;

    let mut holder = pool.handle();
    let mut waiter1 = pool.handle();
    let mut waiter2 = pool.handle();

    let held_lease = holder.acquire().await?;

    let waiter1_task = tokio::spawn(async move { waiter1.acquire().await });
    let waiter2_task = tokio::spawn(async move { waiter2.acquire().await });

    tokio::time::sleep(Duration::from_millis(25)).await;

    waiter1_task.abort();
    let _ = waiter1_task.await;

    drop(held_lease);

    let lease2 = timeout(Duration::from_millis(100), waiter2_task).await???;
    drop(lease2);

    let mut later = pool.handle();
    let _lease3 = timeout(Duration::from_millis(100), later.acquire()).await??;

    Ok(())
}

#[rstest]
#[tokio::test]
async fn handle_path_preserves_warm_reuse_and_preamble(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let (server, pool, preamble_callback_count) =
        build_preamble_pool(client_pool_config.pool_size(1)).await?;
    let mut handle = pool.handle();

    let first: Pong = handle.call(&Ping(7)).await?;
    let second: Pong = handle.call(&Ping(8)).await?;

    assert_eq!(first, Pong(7));
    assert_eq!(second, Pong(8));
    assert_eq!(preamble_callback_count.load(Ordering::SeqCst), 1);
    assert_eq!(server.preamble_count(), 1);
    assert_eq!(server.connection_count(), 1);
    Ok(())
}

#[rstest]
#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn handle_path_recycles_after_idle_timeout(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let idle_timeout = Duration::from_millis(50);
    let (server, pool, preamble_callback_count) =
        build_preamble_pool(client_pool_config.pool_size(1).idle_timeout(idle_timeout)).await?;
    let mut handle = pool.handle();

    let first: Pong = handle.call(&Ping(1)).await?;
    assert_eq!(first, Pong(1));

    advance(idle_timeout + idle_timeout).await;
    tokio::task::yield_now().await;

    let second: Pong = handle.call(&Ping(2)).await?;
    assert_eq!(second, Pong(2));
    assert_eq!(preamble_callback_count.load(Ordering::SeqCst), 2);
    assert_eq!(server.preamble_count(), 2);
    assert_eq!(server.connection_count(), 2);
    Ok(())
}
