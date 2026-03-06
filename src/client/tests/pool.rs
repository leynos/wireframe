//! Unit tests for pooled wireframe clients.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::FutureExt;
use rstest::{fixture, rstest};
use tokio::{
    net::TcpListener,
    task::JoinHandle,
    time::{advance, timeout},
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::{
    client::{ClientPoolConfig, WireframeClient},
    preamble::{read_preamble, write_preamble},
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct ClientHello {
    version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct ServerAck {
    accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct Ping(u8);

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct Pong(u8);

struct PoolTestServer {
    addr: SocketAddr,
    preamble_count: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

async fn handle_pool_test_connection(
    mut stream: tokio::net::TcpStream,
    preamble_count: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
) {
    connection_count.fetch_add(1, Ordering::SeqCst);
    let preamble = read_preamble::<_, ClientHello>(&mut stream).await;
    let Ok((_hello, _leftover)) = preamble else {
        return;
    };
    preamble_count.fetch_add(1, Ordering::SeqCst);
    if write_preamble(&mut stream, &ServerAck { accepted: true })
        .await
        .is_err()
    {
        return;
    }

    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    loop {
        let frame = futures::StreamExt::next(&mut framed).await;
        let Some(Ok(bytes)) = frame else {
            break;
        };
        let decoded = bincode::decode_from_slice::<Ping, _>(&bytes, bincode::config::standard());
        let Ok((ping, _)) = decoded else {
            break;
        };
        let encoded = bincode::encode_to_vec(Pong(ping.0), bincode::config::standard());
        let Ok(payload) = encoded else {
            break;
        };
        if futures::SinkExt::send(&mut framed, payload.into())
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn run_pool_test_accept_loop(
    listener: TcpListener,
    preamble_count: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
) {
    loop {
        let accept_result = listener.accept().await;
        let Ok((stream, _)) = accept_result else {
            break;
        };
        tokio::spawn(handle_pool_test_connection(
            stream,
            preamble_count.clone(),
            connection_count.clone(),
        ));
    }
}

async fn read_server_ack(stream: &mut tokio::net::TcpStream) -> std::io::Result<Vec<u8>> {
    let (ack, leftover) = read_preamble::<_, ServerAck>(stream)
        .await
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if ack.accepted {
        Ok(leftover)
    } else {
        Err(std::io::Error::other("server rejected preamble"))
    }
}

impl PoolTestServer {
    async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let preamble_count = Arc::new(AtomicUsize::new(0));
        let connection_count = Arc::new(AtomicUsize::new(0));
        let server_preamble_count = preamble_count.clone();
        let server_connection_count = connection_count.clone();

        let handle = tokio::spawn(run_pool_test_accept_loop(
            listener,
            server_preamble_count,
            server_connection_count,
        ));

        Ok(Self {
            addr,
            preamble_count,
            connection_count,
            handle,
        })
    }

    fn preamble_count(&self) -> usize { self.preamble_count.load(Ordering::SeqCst) }

    fn connection_count(&self) -> usize { self.connection_count.load(Ordering::SeqCst) }
}

impl Drop for PoolTestServer {
    fn drop(&mut self) { self.handle.abort(); }
}

#[rustfmt::skip]
#[fixture]
fn client_pool_config() -> ClientPoolConfig {
    ClientPoolConfig::default()
}

fn build_pooled_client(
    addr: SocketAddr,
    config: ClientPoolConfig,
    preamble_callback_count: Arc<AtomicUsize>,
) -> impl std::future::Future<
    Output = Result<
        crate::client::WireframeClientPool<crate::serializer::BincodeSerializer, ClientHello, ()>,
        crate::client::ClientError,
    >,
> {
    WireframeClient::builder()
        .with_preamble(ClientHello { version: 1 })
        .on_preamble_success(move |_preamble, stream| {
            let preamble_callback_count = preamble_callback_count.clone();
            async move {
                preamble_callback_count.fetch_add(1, Ordering::SeqCst);
                read_server_ack(stream).await
            }
            .boxed()
        })
        .connect_pool(addr, config)
}

async fn run_ping_round_trip(
    pool: &crate::client::WireframeClientPool<
        crate::serializer::BincodeSerializer,
        ClientHello,
        (),
    >,
    ping: u8,
) -> Result<(), crate::client::ClientError> {
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
