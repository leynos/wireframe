//! `ClientPoolWorld` fixture for rstest-bdd pooled-client scenarios.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::{FutureExt, SinkExt, StreamExt};
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    client::{ClientPoolConfig, WireframeClient, WireframeClientPool},
    preamble::{read_preamble, write_preamble},
    serializer::BincodeSerializer,
};
pub use wireframe_testing::TestResult;

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct ClientHello {
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

struct PoolServer {
    addr: SocketAddr,
    preamble_count: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

async fn handle_pool_connection(
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
        let frame = framed.next().await;
        let Some(Ok(bytes)) = frame else {
            break;
        };
        let decoded = bincode::decode_from_slice::<Ping, _>(&bytes, bincode::config::standard());
        let Ok((ping, _)) = decoded else {
            break;
        };
        let payload = bincode::encode_to_vec(Pong(ping.0), bincode::config::standard());
        let Ok(payload) = payload else {
            break;
        };
        if framed.send(payload.into()).await.is_err() {
            break;
        }
    }
}

async fn run_pool_accept_loop(
    listener: TcpListener,
    preamble_count: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
) {
    loop {
        let accept_result = listener.accept().await;
        let Ok((stream, _)) = accept_result else {
            break;
        };
        tokio::spawn(handle_pool_connection(
            stream,
            preamble_count.clone(),
            connection_count.clone(),
        ));
    }
}

async fn read_ack(stream: &mut tokio::net::TcpStream) -> std::io::Result<Vec<u8>> {
    let (ack, leftover) = read_preamble::<_, ServerAck>(stream)
        .await
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if ack.accepted {
        Ok(leftover)
    } else {
        Err(std::io::Error::other("server rejected preamble"))
    }
}

impl PoolServer {
    async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let preamble_count = Arc::new(AtomicUsize::new(0));
        let connection_count = Arc::new(AtomicUsize::new(0));
        let server_preamble_count = preamble_count.clone();
        let server_connection_count = connection_count.clone();
        let handle = tokio::spawn(run_pool_accept_loop(
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
}

impl Drop for PoolServer {
    fn drop(&mut self) { self.handle.abort(); }
}

pub struct ClientPoolWorld {
    pool: Option<WireframeClientPool<BincodeSerializer, ClientHello, ()>>,
    server: Option<PoolServer>,
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
        self.server = Some(PoolServer::start().await?);
        Ok(())
    }

    async fn connect_pool(&mut self, config: ClientPoolConfig) -> TestResult {
        let addr = self.server.as_ref().ok_or("server missing")?.addr;
        let preamble_callback_count = self.preamble_callback_count.clone();
        let pool = WireframeClient::builder()
            .with_preamble(ClientHello { version: 1 })
            .on_preamble_success(move |_preamble, stream| {
                let preamble_callback_count = preamble_callback_count.clone();
                async move {
                    preamble_callback_count.fetch_add(1, Ordering::SeqCst);
                    read_ack(stream).await
                }
                .boxed()
            })
            .connect_pool(addr, config)
            .await?;
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
        self.recovered_after_release = timeout(Duration::from_millis(100), pool.acquire())
            .await
            .is_ok();
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
                server.preamble_count.load(Ordering::SeqCst) == 1
                    && server.connection_count.load(Ordering::SeqCst) == 1
            })
    }

    pub fn blocked_third_acquire(&self) -> bool { self.blocked_third_acquire }

    pub fn recovered_after_release(&self) -> bool { self.recovered_after_release }

    pub fn reconnected_after_idle(&self) -> bool {
        self.reconnected_after_idle
            && self.preamble_callback_count.load(Ordering::SeqCst) == 2
            && self.server.as_ref().is_some_and(|server| {
                server.preamble_count.load(Ordering::SeqCst) == 2
                    && server.connection_count.load(Ordering::SeqCst) == 2
            })
    }
}
