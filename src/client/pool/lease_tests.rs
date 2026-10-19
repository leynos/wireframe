//! Cancellation and split-phase regression tests for pooled-client leases.

use std::{
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use rstest::rstest;
use tokio::time::{advance, timeout};

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

    // Hold the socket so the lease's own receive has to wait for it, then
    // cancel that checkout. The pending reply must still belong to the lease
    // afterwards, which only holds if checkout failure preserves the gate.
    let slot = lease.slot().ok_or("lease slot must resolve")?;
    let held = slot.pool.get().await?;
    assert!(
        timeout(Duration::from_millis(50), lease.receive::<Pong>())
            .await
            .is_err(),
        "receive must wait while the socket is checked out elsewhere"
    );
    assert!(
        lease.pending_response_permit.lock().await.is_some(),
        "cancelled checkout must keep the pending-response reservation"
    );

    drop(held);
    server.release_delayed_first_response();
    timeout(GRACE, server.wait_for_delayed_first_response_attempt()).await?;

    let response: Pong = timeout(GRACE, lease.receive()).await??;
    assert_eq!(response, Pong(1));
    assert!(
        lease.pending_response_permit.lock().await.is_none(),
        "consuming the reply must release the reservation"
    );

    Ok(())
}

/// Regression for the idle-recycle race `CodeRabbit` flagged on PR #764.
///
/// A successful split-phase `send` disarms the checkout guard and stamps
/// `last_returned_at`. If the caller then waits longer than `idle_timeout`
/// before `receive`, a checkout that recycled on idleness would replace the
/// very socket holding the reply. The reply must survive, and the warm socket
/// must be reused rather than reconnected.
#[rstest]
#[tokio::test(start_paused = true)]
async fn idle_timeout_does_not_recycle_while_a_response_is_pending() -> TestResult {
    let idle_timeout = Duration::from_millis(50);
    let server = PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?;
    let pool = build_pooled_client(
        server.addr,
        ClientPoolConfig::default()
            .pool_size(1)
            .max_in_flight_per_socket(1)
            .idle_timeout(idle_timeout),
        Arc::new(AtomicUsize::new(0)),
    )
    .await?;

    let lease = pool.acquire().await?;
    lease.send(&Ping(7)).await?;
    timeout(GRACE, server.wait_for_delayed_first_response()).await?;
    server.release_delayed_first_response();
    timeout(GRACE, server.wait_for_delayed_first_response_attempt()).await?;

    // The reply is already sitting in the socket buffer, but the lease has not
    // read it yet. Advance the paused clock well past the idle timeout.
    advance(idle_timeout + idle_timeout).await;
    tokio::task::yield_now().await;

    let response: Pong = timeout(GRACE, lease.receive()).await??;
    assert_eq!(response, Pong(7));
    assert_eq!(
        server.connection_count(),
        1,
        "a pending reply must keep the warm socket instead of recycling it"
    );

    Ok(())
}

/// A cancelled in-flight operation must invalidate the socket it was using.
#[rstest]
#[tokio::test]
async fn cancelling_in_flight_call_reconnects_before_reuse() -> TestResult {
    let server = PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?;
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let pool = build_pooled_client(
        server.addr,
        ClientPoolConfig::default()
            .pool_size(1)
            .max_in_flight_per_socket(1),
        preamble_callback_count.clone(),
    )
    .await?;

    let lease = pool.acquire().await?;
    let call_task = tokio::spawn(async move { lease.call::<Ping, Pong>(&Ping(1)).await });
    timeout(GRACE, server.wait_for_delayed_first_response()).await?;

    call_task.abort();
    let join_error = timeout(GRACE, call_task)
        .await?
        .expect_err("cancelled pooled call should stop its task");
    assert!(join_error.is_cancelled());

    server.release_delayed_first_response();
    timeout(GRACE, server.wait_for_delayed_first_response_attempt()).await?;

    let lease = timeout(GRACE, pool.acquire()).await??;
    let response: Pong = lease.call(&Ping(2)).await?;
    assert_eq!(response, Pong(2));

    assert_eq!(
        preamble_callback_count.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(server.preamble_count(), 2);
    assert_eq!(server.connection_count(), 2);

    Ok(())
}

/// A lease dropped between `send` and `receive` must not hand its late reply
/// to another lease.
#[rstest]
#[tokio::test]
async fn dropping_lease_with_unread_split_phase_response_reconnects() -> TestResult {
    let server = PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?;
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let pool = build_pooled_client(
        server.addr,
        ClientPoolConfig::default()
            .pool_size(1)
            .max_in_flight_per_socket(1),
        preamble_callback_count.clone(),
    )
    .await?;

    let lease = pool.acquire().await?;
    lease.send(&Ping(1)).await?;
    timeout(GRACE, server.wait_for_delayed_first_response()).await?;
    drop(lease);

    server.release_delayed_first_response();
    timeout(GRACE, server.wait_for_delayed_first_response_attempt()).await?;

    let lease = timeout(GRACE, pool.acquire()).await??;
    let response: Pong = lease.call(&Ping(2)).await?;
    assert_eq!(response, Pong(2));

    lease.send(&Ping(3)).await?;
    let split_phase_response: Pong = lease.receive().await?;
    assert_eq!(split_phase_response, Pong(3));
    drop(lease);

    let lease = timeout(GRACE, pool.acquire()).await??;
    let response: Pong = lease.call(&Ping(4)).await?;
    assert_eq!(response, Pong(4));

    assert_eq!(
        preamble_callback_count.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(server.preamble_count(), 2);
    assert_eq!(server.connection_count(), 2);

    Ok(())
}
