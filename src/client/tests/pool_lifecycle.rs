//! Client pool lifecycle and slot rotation coverage.
//!
//! Each test here exists to kill a specific surviving mutant from the first
//! full mutation-testing run (issue #593): shutdown refusal, round-robin slot
//! rotation, and draining waiters that are queued when the pool closes.

use std::{
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use rstest::{fixture, rstest};
use tokio::time::timeout;

use crate::{
    client::{ClientError, ClientPoolConfig, PoolTaskChurnObserver},
    test_helpers::{Ping, Pong, PoolTestServer, TestClientPool, build_pooled_client},
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Bound on pool operations that must resolve promptly.
///
/// These operations are expected to complete almost immediately; the bound
/// only exists so a survivor that leaves a caller blocked fails the test
/// instead of hanging the suite.
const PROMPT: Duration = Duration::from_secs(1);

#[rustfmt::skip]
#[fixture]
fn client_pool_config() -> ClientPoolConfig {
    ClientPoolConfig::default()
}

/// Report whether `error` is the typed closure error the pool reports.
fn is_pool_closed(error: &ClientError) -> bool { matches!(error, ClientError::PoolClosed) }

/// Await an acquisition a closed pool must reject, asserting how it failed.
async fn expect_pool_closed<F, T>(acquisition: F) -> TestResult
where
    F: std::future::Future<Output = Result<T, ClientError>>,
{
    let result = timeout(PROMPT, acquisition)
        .await
        .map_err(|_| "a closed pool must resolve pending acquisitions promptly")?;
    let Err(error) = result else {
        return Err("a closed pool must refuse the acquisition".into());
    };
    if is_pool_closed(&error) {
        Ok(())
    } else {
        Err(format!("closed pool must report PoolClosed, got {error:?}").into())
    }
}

/// Send one ping through a fresh lease and assert its echo.
async fn run_ping_round_trip(pool: &TestClientPool, ping: u8) -> Result<(), ClientError> {
    let lease = pool.acquire().await?;
    let reply: Pong = lease.call(&Ping(ping)).await?;
    assert_eq!(reply, Pong(ping));
    Ok(())
}

/// Wait for the fixture to observe `expected` serviced connections.
///
/// The fixture increments its counter after writing a reply, so the client can
/// observe that reply marginally first. A timeout is reported through the
/// returned count, which the caller asserts on, so a missing increment fails
/// the assertion instead of hanging the suite.
async fn wait_for_serviced_connections(server: &PoolTestServer, expected: usize) -> usize {
    let wait = async {
        while server.serviced_connection_count() < expected {
            tokio::task::yield_now().await;
        }
    };
    let _ = timeout(PROMPT, wait).await;
    server.serviced_connection_count()
}

/// Wait until the pool reports `expected` acquisitions parked on capacity.
///
/// Polling this counter is what makes the shutdown test meaningful: closing
/// the pool before both acquisitions reached the wait path would let them fail
/// at the initial shutdown check, and the test would pass even with
/// `notify_shutdown()` removed. The counter is recorded only after that check
/// and only when no slot had a free permit, so a reading of two proves both
/// spawned acquisitions genuinely parked. Returns the observed delta on
/// timeout so the caller can assert on it.
async fn wait_for_capacity_waits(observer: &PoolTaskChurnObserver, expected: u64) -> u64 {
    let wait = async {
        loop {
            let waits = observer.snapshot().capacity_waits;
            if waits >= expected {
                return waits;
            }
            tokio::task::yield_now().await;
        }
    };
    match timeout(PROMPT, wait).await {
        Ok(waits) => waits,
        Err(_) => observer.snapshot().capacity_waits,
    }
}

/// Kills the `is_shutdown` mutant: a closed pool must refuse acquisitions even
/// while an outstanding handle keeps the shared pool state alive.
///
/// The handle keeps a command sender alive, so closing the pool does not close
/// the mailbox underneath it. The shared lifecycle record is what tells the
/// handle the pool is gone. A mutant that always reports "still running" hands
/// out a lease instead.
#[rstest]
#[tokio::test]
async fn close_refuses_later_handle_acquisitions(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let server = PoolTestServer::start().await?;
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1),
        Arc::new(AtomicUsize::new(0)),
    )
    .await?;

    let mut handle = pool.handle();
    pool.close().await;

    expect_pool_closed(handle.acquire()).await
}

/// Kills the `ordered_slots` rotation mutant: round-robin acquisition must
/// spread work across both physical slots.
///
/// Each acquisition and drop advances the slot cursor, so successive round
/// trips use slot 0 and slot 1 in turn. Correct rotation serves frames on both
/// connections; the mutant leaves every acquisition on slot 0, which serves
/// all four replies alone and leaves the other connection silent.
#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn slot_rotation_serves_each_pool_connection(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let server = PoolTestServer::start().await?;
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(2).max_in_flight_per_socket(1),
        Arc::new(AtomicUsize::new(0)),
    )
    .await?;

    for ping in 1..=4 {
        run_ping_round_trip(&pool, ping).await?;
        tokio::task::yield_now().await;
    }

    assert_eq!(
        wait_for_serviced_connections(&server, 2).await,
        2,
        "round-robin acquisition must dispatch work on both pool slots"
    );
    assert_eq!(
        server.connection_count(),
        2,
        "both configured slots must establish a physical connection"
    );
    Ok(())
}

/// Kills the `notify_shutdown` mutant: a waiter still queued when the pool
/// closes must be resolved, not left blocked forever.
///
/// The only permit is held for the whole test, so both acquisitions must park
/// in the scheduler's waiter queue. Once shutdown is published the scheduler
/// stops granting and drains its queue, so only the shutdown drain can
/// complete them. Removing the drain leaves both blocked until the timeout
/// fails this test.
#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn shutdown_drains_a_queued_waiter(client_pool_config: ClientPoolConfig) -> TestResult {
    let server = PoolTestServer::start().await?;
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1).max_in_flight_per_socket(1),
        Arc::new(AtomicUsize::new(0)),
    )
    .await?;

    // Hold the only permit so both acquisitions below have to queue.
    let held_lease = pool.acquire().await?;
    let churn = pool.task_churn_observer();
    let mut first_handle = pool.handle();
    let mut second_handle = pool.handle();

    let first = tokio::spawn(async move { first_handle.acquire().await.map(|_lease| ()) });
    let second = tokio::spawn(async move { second_handle.acquire().await.map(|_lease| ()) });

    // Do not close until both acquisitions have recorded a capacity wait;
    // otherwise either could fail at the initial shutdown check and this test
    // would not exercise the drain path.
    assert_eq!(
        wait_for_capacity_waits(&churn, 2).await,
        2,
        "both acquisitions must be parked on capacity before shutdown"
    );

    pool.close().await;

    let first_result = timeout(PROMPT, first).await??;
    let second_result = timeout(PROMPT, second).await??;

    let first_error = first_result.expect_err("first waiter must fail once the pool closes");
    let second_error = second_result.expect_err("second waiter must fail once the pool closes");
    assert!(
        is_pool_closed(&first_error),
        "first waiter must report PoolClosed, got {first_error:?}"
    );
    assert!(
        is_pool_closed(&second_error),
        "second waiter must report PoolClosed, got {second_error:?}"
    );

    drop(held_lease);
    Ok(())
}
