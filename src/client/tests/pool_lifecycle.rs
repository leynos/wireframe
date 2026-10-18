//! Client pool lifecycle and slot rotation coverage.
//!
//! Each test here exists to kill a specific surviving mutant from the first
//! full mutation-testing run (issue #593): shutdown refusal, round-robin slot
//! rotation, and draining waiters that are queued when the pool closes.

use std::{
    io,
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use rstest::{fixture, rstest};
use tokio::time::timeout;

use crate::{
    WireframeError,
    client::{ClientError, ClientPoolConfig},
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

/// Report whether `error` is the pool's disconnection error.
fn is_disconnected(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Wireframe(WireframeError::Io(source))
            if source.kind() == io::ErrorKind::UnexpectedEof
    )
}

/// Await an acquisition a closed pool must reject, asserting how it failed.
async fn expect_disconnected<F, T>(acquisition: F) -> TestResult
where
    F: std::future::Future<Output = Result<T, ClientError>>,
{
    let result = timeout(PROMPT, acquisition)
        .await
        .map_err(|_| "a closed pool must resolve pending acquisitions promptly")?;
    let Err(error) = result else {
        return Err("a closed pool must refuse the acquisition".into());
    };
    if is_disconnected(&error) {
        Ok(())
    } else {
        Err(format!("closed pool must report disconnection, got {error:?}").into())
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

/// Wait until the scheduler reports one servicing and one queued waiter.
///
/// Polling observable scheduler state is what makes the shutdown test
/// meaningful: closing the pool before both acquisitions are enqueued would
/// let them fail at the initial shutdown check, and the test would pass even
/// with `notify_shutdown()` removed. Returns the observed counts on timeout so
/// the caller can assert on them.
async fn wait_for_waiter_counts(pool: &TestClientPool) -> (usize, usize) {
    let wait = async {
        loop {
            let counts = pool.waiter_counts();
            if counts == (1, 1) {
                return counts;
            }
            tokio::task::yield_now().await;
        }
    };
    match timeout(PROMPT, wait).await {
        Ok(counts) => counts,
        Err(_) => pool.waiter_counts(),
    }
}

/// Kills the `is_shutdown` mutant: a closed pool must refuse acquisitions even
/// while an outstanding handle keeps the shared pool state alive.
///
/// The handle owns a strong reference, so `close()` cannot drop the state; the
/// shutdown flag is the only thing that tells the handle the pool is gone. A
/// mutant that always reports "not shut down" hands out a lease instead.
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

    expect_disconnected(handle.acquire()).await
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
/// The first waiter is claimed by the background service task and parks on
/// slot capacity; the second stays in the scheduler queue. The test waits for
/// those exact scheduler counts before closing, so both acquisitions are
/// genuinely past the initial shutdown check. Only `notify_shutdown()` drains
/// the queued waiter, so removing it leaves the second acquire blocked until
/// the timeout fails this test.
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
    let mut claimed_handle = pool.handle();
    let mut queued_handle = pool.handle();

    let claimed = tokio::spawn(async move { claimed_handle.acquire().await.map(|_lease| ()) });
    let queued = tokio::spawn(async move { queued_handle.acquire().await.map(|_lease| ()) });

    // Do not close until the scheduler itself shows one claimed waiter parked
    // on capacity and one still queued; otherwise both could fail at the
    // initial shutdown check and this test would not exercise the drain path.
    assert_eq!(
        wait_for_waiter_counts(&pool).await,
        (1, 1),
        "one waiter must be servicing and one queued before shutdown"
    );

    pool.close().await;

    let claimed_result = timeout(PROMPT, claimed).await??;
    let queued_result = timeout(PROMPT, queued).await??;

    let claimed_error = claimed_result.expect_err("claimed waiter must fail once the pool closes");
    let queued_error = queued_result.expect_err("queued waiter must fail once the pool closes");
    assert!(
        is_disconnected(&claimed_error),
        "claimed waiter must report disconnection, got {claimed_error:?}"
    );
    assert!(
        is_disconnected(&queued_error),
        "queued waiter must report disconnection, got {queued_error:?}"
    );

    drop(held_lease);
    Ok(())
}
