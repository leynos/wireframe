//! `ClientPoolWorld` fixture for rstest-bdd pooled-client scenarios.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rstest::fixture;
use tokio::time::timeout;
use wireframe::{
    client::ClientPoolConfig,
    test_helpers::{Ping, Pong, PoolTestServer, TestClientPool, build_pooled_client},
};
pub use wireframe_testing::TestResult;

pub struct ClientPoolWorld {
    pool: Option<TestClientPool>,
    server: Option<PoolTestServer>,
    preamble_callback_count: Arc<AtomicUsize>,
    blocked_third_acquire: bool,
    recovered_after_release: bool,
    reconnected_after_idle: bool,
}

impl Default for ClientPoolWorld {
    fn default() -> Self {
        Self {
            pool: None,
            server: None,
            preamble_callback_count: Arc::new(AtomicUsize::new(0)),
            blocked_third_acquire: false,
            recovered_after_release: false,
            reconnected_after_idle: false,
        }
    }
}

#[rustfmt::skip]
#[fixture]
pub fn client_pool_world() -> ClientPoolWorld {
    ClientPoolWorld::default()
}

impl ClientPoolWorld {
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

    pub async fn run_warm_reuse_scenario(&mut self) -> TestResult {
        self.start_server().await?;
        self.connect_pool(ClientPoolConfig::default().pool_size(1))
            .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;

        let lease = pool.acquire().await?;
        let first: Pong = lease.call(&Ping(7)).await?;
        if first != Pong(7) {
            return Err(format!("unexpected first response: {first:?}").into());
        }
        drop(lease);

        let lease = pool.acquire().await?;
        let second: Pong = lease.call(&Ping(8)).await?;
        if second != Pong(8) {
            return Err(format!("unexpected second response: {second:?}").into());
        }

        Ok(())
    }

    pub async fn run_in_flight_limit_scenario(&mut self) -> TestResult {
        self.start_server().await?;
        self.connect_pool(
            ClientPoolConfig::default()
                .pool_size(1)
                .max_in_flight_per_socket(2),
        )
        .await?;
        let pool = self.pool.as_ref().ok_or("pool missing")?;

        let first = pool.acquire().await?;
        let second = pool.acquire().await?;
        self.blocked_third_acquire = timeout(Duration::from_millis(25), pool.acquire())
            .await
            .is_err();

        drop(first);
        self.recovered_after_release = matches!(
            timeout(Duration::from_millis(100), pool.acquire()).await,
            Ok(Ok(_))
        );
        drop(second);
        Ok(())
    }

    pub async fn run_idle_recycle_scenario(&mut self) -> TestResult {
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

        let lease = pool.acquire().await?;
        let first: Pong = lease.call(&Ping(1)).await?;
        if first != Pong(1) {
            return Err(format!("unexpected first response: {first:?}").into());
        }
        drop(lease);

        tokio::time::advance(idle_timeout + idle_timeout).await;
        tokio::task::yield_now().await;

        let lease = pool.acquire().await?;
        let second: Pong = lease.call(&Ping(2)).await?;
        if second != Pong(2) {
            return Err(format!("unexpected second response: {second:?}").into());
        }
        self.reconnected_after_idle = true;
        Ok(())
    }

    pub fn warm_reuse_preserved(&self) -> bool {
        self.preamble_callback_count.load(Ordering::SeqCst) == 1
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count() == 1 && server.connection_count() == 1
            })
    }

    pub fn blocked_third_acquire(&self) -> bool { self.blocked_third_acquire }

    pub fn recovered_after_release(&self) -> bool { self.recovered_after_release }

    pub fn reconnected_after_idle(&self) -> bool {
        self.reconnected_after_idle
            && self.preamble_callback_count.load(Ordering::SeqCst) == 2
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count() == 2 && server.connection_count() == 2
            })
    }
}
