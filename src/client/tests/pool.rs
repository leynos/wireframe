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

#[rstest]
#[tokio::test]
async fn pooled_reuse_preserves_preamble_state(client_pool_config: ClientPoolConfig) -> TestResult {
    let server = PoolTestServer::start().await?;
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1),
        preamble_callback_count.clone(),
    )
    .await?;

    let lease = pool.acquire().await?;
    let first: Pong = lease.call(&Ping(7)).await?;
    assert_eq!(first, Pong(7));
    drop(lease);

    let lease = pool.acquire().await?;
    let second: Pong = lease.call(&Ping(8)).await?;
    assert_eq!(second, Pong(8));

    assert_eq!(preamble_callback_count.load(Ordering::SeqCst), 1);
    assert_eq!(server.preamble_count(), 1);
    assert_eq!(server.connection_count(), 1);
    Ok(())
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
    let server = PoolTestServer::start().await?;
    let preamble_callback_count = Arc::new(AtomicUsize::new(0));
    let idle_timeout = Duration::from_millis(50);
    let pool = build_pooled_client(
        server.addr,
        client_pool_config.pool_size(1).idle_timeout(idle_timeout),
        preamble_callback_count.clone(),
    )
    .await?;

    let lease = pool.acquire().await?;
    let first: Pong = lease.call(&Ping(1)).await?;
    assert_eq!(first, Pong(1));
    drop(lease);

    advance(idle_timeout + idle_timeout).await;
    tokio::task::yield_now().await;

    let lease = pool.acquire().await?;
    let second: Pong = lease.call(&Ping(2)).await?;
    assert_eq!(second, Pong(2));

    assert_eq!(preamble_callback_count.load(Ordering::SeqCst), 2);
    assert_eq!(server.preamble_count(), 2);
    assert_eq!(server.connection_count(), 2);
    Ok(())
}
