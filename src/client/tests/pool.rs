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

use super::helpers::test_with_client;
use crate::{
    client::{ClientError, ClientPoolConfig, pool::managed::ManagedClientConnection},
    serializer::BincodeSerializer,
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
type ManagedFixture = (
    ManagedClientConnection<BincodeSerializer, ()>,
    Arc<AtomicUsize>,
);

/// Upper bound on legitimate pooled teardown, so a stuck drop fails loudly
/// rather than hanging the suite.
const GRACE: Duration = Duration::from_secs(1);

/// Keep teardown failures attributed to the test that dropped the connection.
#[track_caller]
fn assert_teardown_once(teardown_count: &AtomicUsize) {
    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        1,
        "pooled connection teardown callback must run exactly once"
    );
}

/// Build a real connected resource so its teardown path exercises socket close
/// and the callback together.
#[fixture]
async fn managed_connection_with_teardown() -> super::helpers::TestResult<ManagedFixture> {
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let teardown_callback_count = Arc::clone(&teardown_count);
    let client = test_with_client(|builder| {
        builder
            .on_connection_setup(|| async {})
            .on_connection_teardown(move |()| {
                let callback_count = Arc::clone(&teardown_callback_count);
                async move {
                    callback_count.fetch_add(1, Ordering::SeqCst);
                }
            })
    })
    .await?;
    Ok((ManagedClientConnection::new(client), teardown_count))
}

/// Where the pooled connection is dropped, relative to the test's runtime.
#[derive(Debug, Clone, Copy)]
enum DropSite {
    /// Drop inline, while the test's Tokio runtime is still driving.
    ActiveRuntime,
    /// Drop on a spawned thread, which owns no Tokio runtime.
    DetachedThread,
}

/// How to wait for a drop that has been performed.
enum DropCompletion {
    /// The drop ran inline on the test thread, so it has already finished.
    Inline,
    /// The drop runs on a detached thread that signals when it completes.
    ///
    /// Joining that thread blocks the caller, and `#[tokio::test]` drives a
    /// single-threaded runtime, so joining inline would starve the executor and
    /// stop any later `timeout` from arming. The worker signals through a
    /// channel instead, letting the caller bound the wait, and is joined only
    /// once it has finished.
    Detached {
        worker: std::thread::JoinHandle<()>,
        finished: tokio::sync::oneshot::Receiver<()>,
    },
}

impl DropCompletion {
    /// Wait for the drop to finish, failing rather than hanging past `grace`.
    ///
    /// Waiting on the channel first is what makes a stuck teardown observable:
    /// the caller fails on the elapsed timeout instead of blocking forever in a
    /// join. The worker is joined afterwards, once it has already finished, so
    /// that a panic inside it is still reported against this test.
    async fn wait(self, grace: Duration) -> TestResult {
        let Self::Detached { worker, finished } = self else {
            return Ok(());
        };
        if timeout(grace, finished).await.is_err() {
            return Err(std::io::Error::other(
                "pooled connection drop did not finish within the grace period",
            )
            .into());
        }
        join_worker(worker)
    }
}

/// Join a worker that has already finished, surfacing a panic from that thread.
fn join_worker(worker: std::thread::JoinHandle<()>) -> TestResult {
    worker
        .join()
        .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
            std::io::Error::other("pooled connection drop thread panicked").into()
        })
}

impl DropSite {
    /// Drop `managed` here, returning how to wait for that drop to finish.
    fn drop_managed(
        self,
        managed: ManagedClientConnection<BincodeSerializer, ()>,
    ) -> DropCompletion {
        match self {
            Self::ActiveRuntime => {
                drop(managed);
                DropCompletion::Inline
            }
            Self::DetachedThread => {
                let (sender, finished) = tokio::sync::oneshot::channel();
                let worker = std::thread::spawn(move || {
                    drop(managed);
                    // A failed send means the caller stopped waiting, which is
                    // not a worker failure.
                    let _ = sender.send(());
                });
                DropCompletion::Detached { worker, finished }
            }
        }
    }
}

/// Teardown runs exactly once whether or not a runtime is driving the drop.
///
/// Both the drop and the teardown signal are awaited under [`GRACE`], so a
/// teardown that never completes fails this test instead of hanging it.
#[rstest]
#[case::active_runtime(DropSite::ActiveRuntime)]
#[case::detached_thread(DropSite::DetachedThread)]
#[tokio::test]
async fn managed_connection_drop_runs_teardown_once(
    #[case] drop_site: DropSite,
    #[future] managed_connection_with_teardown: super::helpers::TestResult<ManagedFixture>,
) -> TestResult {
    let (mut managed, teardown_count) = managed_connection_with_teardown.await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    managed.notify_after_close(sender);

    drop_site.drop_managed(managed).wait(GRACE).await?;
    timeout(GRACE, receiver).await??;
    assert_teardown_once(&teardown_count);
    Ok(())
}

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

#[rstest]
#[tokio::test]
async fn cancelling_in_flight_call_reconnects_before_reuse(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let server = PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?;
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1).max_in_flight_per_socket(1),
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

    assert_eq!(preamble_callback_count.load(Ordering::SeqCst), 2);
    assert_eq!(server.preamble_count(), 2);
    assert_eq!(server.connection_count(), 2);

    Ok(())
}

#[rstest]
#[tokio::test]
async fn dropping_lease_with_unread_split_phase_response_reconnects(
    client_pool_config: ClientPoolConfig,
) -> TestResult {
    let server = PoolTestServer::start_with_behavior(PoolServerBehavior::LateFirstResponse).await?;
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1).max_in_flight_per_socket(1),
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
    assert_eq!(preamble_callback_count.load(Ordering::SeqCst), 2);
    assert_eq!(server.preamble_count(), 2);
    assert_eq!(server.connection_count(), 2);

    Ok(())
}
