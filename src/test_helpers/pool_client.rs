//! Shared pooled-client test helpers.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use futures::{FutureExt, SinkExt, StreamExt};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::{
    client::{ClientError, ClientPoolConfig, WireframeClient, WireframeClientPool},
    preamble::{read_preamble, write_preamble},
    serializer::BincodeSerializer,
};

/// Test preamble payload used by pooled-client fixtures.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct ClientHello {
    /// Protocol version sent in the client preamble.
    pub version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct ServerAck {
    accepted: bool,
}

/// Test request message used by pooled-client fixtures.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct Ping(pub u8);

/// Test response message used by pooled-client fixtures.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct Pong(pub u8);

/// Shared pooled-client type used by pool tests and fixtures.
pub type TestClientPool = WireframeClientPool<BincodeSerializer, ClientHello, ()>;

/// Behaviour knobs for the pooled test server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PoolServerBehavior {
    /// Reply with a normal `Pong` frame for every `Ping`.
    #[default]
    EchoPong,
    /// Send one malformed response payload, then resume normal replies.
    MalformedFirstResponse,
}

/// Shared pooled test server used by unit and behavioural tests.
pub struct PoolTestServer {
    /// Socket address the test server is listening on.
    pub addr: SocketAddr,
    preamble_count: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

struct PoolServerState {
    preamble_count: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
    malformed_response_sent: Arc<AtomicBool>,
    behavior: PoolServerBehavior,
}

impl PoolTestServer {
    /// Start the pooled test server with echo behaviour.
    ///
    /// # Errors
    ///
    /// Returns an error if the test listener cannot bind to a local port.
    pub async fn start() -> std::io::Result<Self> {
        Self::start_with_behavior(PoolServerBehavior::EchoPong).await
    }

    /// Start the pooled test server with the requested behaviour.
    ///
    /// # Errors
    ///
    /// Returns an error if the test listener cannot bind to a local port.
    pub async fn start_with_behavior(behavior: PoolServerBehavior) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let preamble_count = Arc::new(AtomicUsize::new(0));
        let connection_count = Arc::new(AtomicUsize::new(0));
        let state = PoolServerState {
            preamble_count: preamble_count.clone(),
            connection_count: connection_count.clone(),
            malformed_response_sent: Arc::new(AtomicBool::new(false)),
            behavior,
        };

        let handle = tokio::spawn(run_pool_test_accept_loop(listener, state));

        Ok(Self {
            addr,
            preamble_count,
            connection_count,
            handle,
        })
    }

    /// Return the number of successful preamble exchanges.
    #[must_use]
    pub fn preamble_count(&self) -> usize { self.preamble_count.load(Ordering::SeqCst) }

    /// Return the number of physical connections accepted by the server.
    #[must_use]
    pub fn connection_count(&self) -> usize { self.connection_count.load(Ordering::SeqCst) }
}

impl Drop for PoolTestServer {
    fn drop(&mut self) { self.handle.abort(); }
}

/// Build a pooled client configured for the shared test protocol.
pub fn build_pooled_client(
    addr: SocketAddr,
    config: ClientPoolConfig,
    preamble_callback_count: Arc<AtomicUsize>,
) -> impl std::future::Future<Output = Result<TestClientPool, ClientError>> {
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

/// Acquire and release a lease repeatedly, recording each grant in a shared log.
///
/// # Errors
///
/// Returns an error if any `acquire()` call fails.
pub async fn acquire_and_record(
    mut handle: crate::client::PoolHandle<BincodeSerializer, ClientHello, ()>,
    label: &'static str,
    rounds: usize,
    grants: Arc<Mutex<Vec<&'static str>>>,
) -> Result<(), ClientError> {
    for _ in 0..rounds {
        let lease = handle.acquire().await?;
        grants.lock().await.push(label);
        tokio::task::yield_now().await;
        drop(lease);
    }
    Ok(())
}

async fn run_pool_test_accept_loop(listener: TcpListener, state: PoolServerState) {
    loop {
        let accept_result = listener.accept().await;
        let Ok((stream, _)) = accept_result else {
            break;
        };
        let state = PoolServerState {
            preamble_count: state.preamble_count.clone(),
            connection_count: state.connection_count.clone(),
            malformed_response_sent: state.malformed_response_sent.clone(),
            behavior: state.behavior,
        };
        tokio::spawn(handle_pool_test_connection(stream, state));
    }
}

async fn handle_pool_test_connection(mut stream: tokio::net::TcpStream, state: PoolServerState) {
    state.connection_count.fetch_add(1, Ordering::SeqCst);
    let preamble = read_preamble::<_, ClientHello>(&mut stream).await;
    let Ok((_hello, _leftover)) = preamble else {
        return;
    };
    state.preamble_count.fetch_add(1, Ordering::SeqCst);
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

        let payload = if state.behavior == PoolServerBehavior::MalformedFirstResponse
            && !state.malformed_response_sent.swap(true, Ordering::SeqCst)
        {
            Vec::new()
        } else {
            let encoded = bincode::encode_to_vec(Pong(ping.0), bincode::config::standard());
            let Ok(payload) = encoded else {
                break;
            };
            payload
        };

        if framed.send(payload.into()).await.is_err() {
            break;
        }
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
