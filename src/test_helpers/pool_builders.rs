//! Builders for pooled clients used by unit and behavioural tests.
//!
//! Kept apart from [`super::pool_client`] so the pooled fixture server and the
//! pooled client recipes each stay within the module size limit.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures::FutureExt;
use tokio::sync::mpsc;

use super::pool_client::{ClientHello, PoolTestServer, TestClientPool, read_server_ack};
use crate::{
    client::{ClientError, ClientPoolConfig, WireframeClient, WireframeClientBuilder},
    serializer::BincodeSerializer,
};

/// Connection state recorded by the lifecycle fixture: the generation number
/// assigned to a physical connection when it is established.
pub type LifecycleGeneration = usize;

/// Pool type whose lifecycle hooks record connection generations.
pub type LifecycleTestClientPool =
    crate::client::WireframeClientPool<BincodeSerializer, ClientHello, LifecycleGeneration>;

/// Observable effects of the hooks configured by [`build_lifecycle_pool`].
pub struct LifecycleCounters {
    /// How many setup callbacks have run.
    pub setup_count: Arc<AtomicUsize>,
    /// Generations reported by teardown callbacks, in completion order.
    pub teardown_generations: mpsc::UnboundedReceiver<LifecycleGeneration>,
    /// How many error-hook invocations have run.
    pub error_count: Arc<AtomicUsize>,
}

/// Build the shared preamble handshake for pooled test fixtures.
///
/// The counter records how many connections completed the handshake, which
/// lets callers distinguish a warm socket reuse from a fresh connection.
fn pooled_client_builder(
    preamble_callback_count: Arc<AtomicUsize>,
) -> WireframeClientBuilder<BincodeSerializer, ClientHello, ()> {
    WireframeClient::builder()
        .with_preamble(ClientHello { version: 1 })
        .on_preamble_success(move |_preamble, stream| {
            let callback_count = preamble_callback_count.clone();
            async move {
                callback_count.fetch_add(1, Ordering::SeqCst);
                read_server_ack(stream).await
            }
            .boxed()
        })
}

/// Build a pooled client configured for the shared test protocol.
pub fn build_pooled_client(
    addr: SocketAddr,
    config: ClientPoolConfig,
    preamble_callback_count: Arc<AtomicUsize>,
) -> impl std::future::Future<Output = Result<TestClientPool, ClientError>> {
    pooled_client_builder(preamble_callback_count).connect_pool(addr, config)
}

/// Build a pooled client with a preamble counter for tracking warm reuse.
///
/// # Errors
///
/// Returns an error if the server cannot be started or the pool connection fails.
pub async fn build_preamble_pool(
    config: ClientPoolConfig,
) -> Result<(PoolTestServer, TestClientPool, Arc<AtomicUsize>), ClientError> {
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let server = PoolTestServer::start().await?;
    let pool = build_pooled_client(server.addr, config, preamble_callback_count.clone()).await?;
    Ok((server, pool, preamble_callback_count))
}

/// Build a pooled client whose lifecycle hooks record connection generations.
///
/// Setup assigns each new physical connection the next generation number and
/// stores it as that connection's state. Teardown reports the generation it
/// was given, so a caller can tell which physical connection was closed. The
/// error hook counts invocations separately.
///
/// # Errors
///
/// Returns an error if the pool cannot be constructed. Connections open
/// lazily, so handshake failures surface on the first lease operation.
pub async fn build_lifecycle_pool(
    addr: SocketAddr,
    config: ClientPoolConfig,
) -> Result<(LifecycleTestClientPool, LifecycleCounters), ClientError> {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let error_count = Arc::new(AtomicUsize::new(0));
    let (teardown_sender, teardown_generations) = mpsc::unbounded_channel();

    // This fixture only needs the handshake to complete; it does not inspect
    // how many preamble callbacks ran.
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let setup_counter = Arc::clone(&setup_count);
    let error_counter = Arc::clone(&error_count);

    let pool = pooled_client_builder(preamble_callback_count)
        .on_connection_setup(move || {
            let counter = Arc::clone(&setup_counter);
            async move {
                // `fetch_add` returns the previous count, so numbering starts
                // at generation 1.
                counter.fetch_add(1, Ordering::SeqCst) + 1
            }
        })
        .on_connection_teardown(move |generation| {
            // A closed receiver means the fixture is gone; losing a report
            // must not panic a pool worker.
            let _send_result = teardown_sender.send(generation);
            async {}
        })
        .on_error(move |_error| {
            let counter = Arc::clone(&error_counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect_pool(addr, config)
        .await?;

    Ok((
        pool,
        LifecycleCounters {
            setup_count,
            teardown_generations,
            error_count,
        },
    ))
}
