//! `ClientPoolHandleWorld` fixture for `PoolHandle` behavioural scenarios.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rstest::fixture;
use tokio::{sync::Mutex, time::timeout};
use wireframe::{
    client::{ClientPoolConfig, PoolFairnessPolicy},
    test_helpers::{Ping, Pong, PoolTestServer, TestClientPool, build_pooled_client},
};
pub use wireframe_testing::TestResult;

pub struct ClientPoolHandleWorld {
    pool: Option<TestClientPool>,
    server: Option<PoolTestServer>,
    preamble_callback_count: Arc<AtomicUsize>,
    grant_order: Vec<String>,
    blocked_waiter: bool,
    waiter_recovered: bool,
    warm_reuse_then_recycle: bool,
}

impl Default for ClientPoolHandleWorld {
    fn default() -> Self {
        Self {
            pool: None,
            server: None,
            preamble_callback_count: Arc::new(AtomicUsize::new(0)),
            grant_order: Vec::new(),
            blocked_waiter: false,
            waiter_recovered: false,
            warm_reuse_then_recycle: false,
        }
    }
}

#[rustfmt::skip]
#[fixture]
pub fn client_pool_handle_world() -> ClientPoolHandleWorld {
    ClientPoolHandleWorld::default()
}

impl ClientPoolHandleWorld {
    async fn connect_handle_pool(&mut self, fairness_policy: PoolFairnessPolicy) -> TestResult {
        self.start_server().await?;
        self.connect_pool(
            ClientPoolConfig::default()
                .pool_size(1)
                .fairness_policy(fairness_policy),
        )
        .await
    }

    async fn record_grants(&mut self, grants: Arc<Mutex<Vec<&'static str>>>) -> TestResult {
        self.grant_order = grants
            .lock()
            .await
            .iter()
            .map(ToString::to_string)
            .collect();
        Ok(())
    }

    async fn start_server(&mut self) -> TestResult {
        self.server = Some(PoolTestServer::start().await?);
        Ok(())
    }

    async fn connect_pool(&mut self, config: ClientPoolConfig) -> TestResult {
        let addr = self.server.as_ref().ok_or("server missing")?.addr;
        let pool = build_pooled_client(addr, config, self.preamble_callback_count.clone()).await?;
        self.pool = Some(pool);
        Ok(())
    }

    pub async fn run_round_robin_scenario(&mut self) -> TestResult {
        self.connect_handle_pool(PoolFairnessPolicy::RoundRobin)
            .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;
        let grants = Arc::new(Mutex::new(Vec::new()));

        let left = tokio::spawn(run_repeated_acquire(
            pool.handle(),
            "session-a",
            3,
            Arc::clone(&grants),
        ));
        let right = tokio::spawn(run_repeated_acquire(
            pool.handle(),
            "session-b",
            3,
            Arc::clone(&grants),
        ));
        left.await??;
        right.await??;
        self.record_grants(grants).await
    }

    pub async fn run_fifo_scenario(&mut self) -> TestResult {
        self.connect_handle_pool(PoolFairnessPolicy::Fifo).await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;
        let blocker = pool.acquire().await?;
        let grants = Arc::new(Mutex::new(Vec::new()));
        let (first, second, third) = spawn_fifo_waiters(pool, Arc::clone(&grants)).await;

        drop(blocker);
        first.await??;
        second.await??;
        third.await??;
        self.record_grants(grants).await
    }

    pub async fn run_back_pressure_scenario(&mut self) -> TestResult {
        self.start_server().await?;
        self.connect_pool(ClientPoolConfig::default().pool_size(1))
            .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;
        let mut first = pool.handle();
        let mut second = pool.handle();

        let held_lease = first.acquire().await?;
        self.blocked_waiter = timeout(Duration::from_millis(25), second.acquire())
            .await
            .is_err();

        drop(held_lease);
        self.waiter_recovered = matches!(
            timeout(Duration::from_millis(100), second.acquire()).await,
            Ok(Ok(_))
        );
        Ok(())
    }

    pub async fn run_warm_reuse_then_idle_recycle_scenario(&mut self) -> TestResult {
        tokio::time::pause();
        self.start_server().await?;
        let idle_timeout = Duration::from_millis(50);
        self.connect_pool(
            ClientPoolConfig::default()
                .pool_size(1)
                .idle_timeout(idle_timeout),
        )
        .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;
        let mut handle = pool.handle();

        let first: Pong = handle.call(&Ping(1)).await?;
        let second: Pong = handle.call(&Ping(2)).await?;
        if first != Pong(1) || second != Pong(2) {
            return Err("unexpected warm reuse response sequence".into());
        }
        let warm_reuse_preserved = self.preamble_callback_count.load(Ordering::SeqCst) == 1
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count() == 1 && server.connection_count() == 1
            });

        tokio::time::advance(idle_timeout + idle_timeout).await;
        tokio::task::yield_now().await;

        let third: Pong = handle.call(&Ping(3)).await?;
        self.warm_reuse_then_recycle = third == Pong(3)
            && warm_reuse_preserved
            && self.preamble_callback_count.load(Ordering::SeqCst) == 2
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count() == 2 && server.connection_count() == 2
            });
        Ok(())
    }

    pub fn sessions_alternate_fairly(&self) -> bool {
        self.grant_order
            == [
                "session-a",
                "session-b",
                "session-a",
                "session-b",
                "session-a",
                "session-b",
            ]
    }

    pub fn fifo_order_preserved(&self) -> bool { self.grant_order == ["first", "second", "third"] }

    pub fn back_pressure_preserved(&self) -> bool { self.blocked_waiter && self.waiter_recovered }

    pub fn warm_reuse_then_recycle_preserved(&self) -> bool { self.warm_reuse_then_recycle }
}

async fn spawn_fifo_waiters(
    pool: &TestClientPool,
    grants: Arc<Mutex<Vec<&'static str>>>,
) -> (
    tokio::task::JoinHandle<Result<(), wireframe::client::ClientError>>,
    tokio::task::JoinHandle<Result<(), wireframe::client::ClientError>>,
    tokio::task::JoinHandle<Result<(), wireframe::client::ClientError>>,
) {
    let first = tokio::spawn(run_repeated_acquire(
        pool.handle(),
        "first",
        1,
        Arc::clone(&grants),
    ));
    tokio::task::yield_now().await;
    let second = tokio::spawn(run_repeated_acquire(
        pool.handle(),
        "second",
        1,
        Arc::clone(&grants),
    ));
    tokio::task::yield_now().await;
    let third = tokio::spawn(run_repeated_acquire(pool.handle(), "third", 1, grants));
    tokio::task::yield_now().await;
    (first, second, third)
}

async fn run_repeated_acquire(
    mut handle: wireframe::client::PoolHandle<
        wireframe::serializer::BincodeSerializer,
        wireframe::test_helpers::ClientHello,
        (),
    >,
    label: &'static str,
    rounds: usize,
    grants: Arc<Mutex<Vec<&'static str>>>,
) -> Result<(), wireframe::client::ClientError> {
    for _ in 0..rounds {
        let lease = handle.acquire().await?;
        grants.lock().await.push(label);
        tokio::task::yield_now().await;
        drop(lease);
    }
    Ok(())
}
