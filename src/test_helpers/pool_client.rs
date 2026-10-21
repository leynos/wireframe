//! Shared pooled-client test helpers.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use futures::{FutureExt, SinkExt, StreamExt};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Notify},
    task::JoinHandle,
};
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
/// Internal server response used to complete the preamble handshake.
struct ServerAck {
    /// Whether the fixture accepts the client's preamble.
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
    /// Hold the first reply until the test releases it, then resume normally.
    ///
    /// Lets a test observe a request that is in flight server-side, so it can
    /// cancel or drop the client operation before the reply is written.
    LateFirstResponse,
}

/// Shared pooled test server used by unit and behavioural tests.
pub struct PoolTestServer {
    /// Socket address the test server is listening on.
    pub addr: SocketAddr,
    /// Count of client preambles decoded by `read_preamble`, regardless of
    /// whether the subsequent `write_preamble` acknowledgement succeeds.
    preamble_count: Arc<AtomicUsize>,
    /// Count of physical TCP connections accepted by the fixture.
    connection_count: Arc<AtomicUsize>,
    /// Count of connections that have served at least one well-formed reply.
    serviced_connection_count: Arc<AtomicUsize>,
    /// Signals that a held first reply is blocking the fixture.
    delayed_first_response_started: Arc<Notify>,
    /// Released by the test once the client operation has been cancelled.
    delayed_first_response_release: Arc<Notify>,
    /// Signals that the fixture has attempted the held reply's write.
    delayed_first_response_attempted: Arc<Notify>,
    /// Background accept loop; aborting it makes dropping the fixture bounded.
    handle: JoinHandle<()>,
}

/// Per-server state shared with each spawned connection task.
struct PoolServerState {
    /// Shared successful-handshake counter.
    preamble_count: Arc<AtomicUsize>,
    /// Shared accepted-connection counter.
    connection_count: Arc<AtomicUsize>,
    /// Shared counter of connections that have served a well-formed reply.
    serviced_connection_count: Arc<AtomicUsize>,
    /// One-shot flag controlling special first-response coverage.
    special_response_handled: Arc<AtomicBool>,
    /// Signals that a held first reply is blocking the fixture.
    delayed_first_response_started: Arc<Notify>,
    /// Released by the test once the client operation has been cancelled.
    delayed_first_response_release: Arc<Notify>,
    /// Signals that the fixture has attempted the held reply's write.
    delayed_first_response_attempted: Arc<Notify>,
    /// Response mode selected by the test case.
    behaviour: PoolServerBehavior,
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
    pub async fn start_with_behavior(behaviour: PoolServerBehavior) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let preamble_count = Arc::new(AtomicUsize::new(0));
        let connection_count = Arc::new(AtomicUsize::new(0));
        let serviced_connection_count = Arc::new(AtomicUsize::new(0));
        let delayed_first_response_started = Arc::new(Notify::new());
        let delayed_first_response_release = Arc::new(Notify::new());
        let delayed_first_response_attempted = Arc::new(Notify::new());
        let state = PoolServerState {
            preamble_count: preamble_count.clone(),
            connection_count: connection_count.clone(),
            serviced_connection_count: serviced_connection_count.clone(),
            special_response_handled: Arc::new(AtomicBool::new(false)),
            delayed_first_response_started: delayed_first_response_started.clone(),
            delayed_first_response_release: delayed_first_response_release.clone(),
            delayed_first_response_attempted: delayed_first_response_attempted.clone(),
            behaviour,
        };

        let handle = tokio::spawn(run_pool_test_accept_loop(listener, state));

        Ok(Self {
            addr,
            preamble_count,
            connection_count,
            serviced_connection_count,
            delayed_first_response_started,
            delayed_first_response_release,
            delayed_first_response_attempted,
            handle,
        })
    }

    /// Return the number of successful preamble exchanges.
    #[must_use]
    pub fn preamble_count(&self) -> usize { self.preamble_count.load(Ordering::SeqCst) }

    /// Return the number of physical connections accepted by the server.
    #[must_use]
    pub fn connection_count(&self) -> usize { self.connection_count.load(Ordering::SeqCst) }

    /// Count connections that served a well-formed reply at least once.
    ///
    /// Unlike [`Self::connection_count`], which counts accepted sockets, this
    /// counts only connections that carried a request through to a successful
    /// reply, so it evidences dispatched work. Malformed replies never count,
    /// and each connection contributes at most once however many frames it
    /// serves.
    #[must_use]
    pub fn serviced_connection_count(&self) -> usize {
        self.serviced_connection_count.load(Ordering::SeqCst)
    }

    /// Wait until [`PoolServerBehavior::LateFirstResponse`] holds a reply.
    pub async fn wait_for_delayed_first_response(&self) {
        self.delayed_first_response_started.notified().await;
    }

    /// Release the reply held by [`PoolServerBehavior::LateFirstResponse`].
    pub fn release_delayed_first_response(&self) {
        self.delayed_first_response_release.notify_one();
    }

    /// Wait until the fixture has attempted the held reply's write.
    ///
    /// A cancelled client closes its socket, so this write may fail; the
    /// signal reports the attempt, not success.
    pub async fn wait_for_delayed_first_response_attempt(&self) {
        self.delayed_first_response_attempted.notified().await;
    }
}

impl Drop for PoolTestServer {
    fn drop(&mut self) {
        // Unblock a detached connection task when a test exits before it has
        // released the held reply.
        self.delayed_first_response_release.notify_one();
        self.handle.abort();
    }
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

/// Accept connections until the listener is closed or its task is aborted.
async fn run_pool_test_accept_loop(listener: TcpListener, state: PoolServerState) {
    loop {
        let accept_result = listener.accept().await;
        let Ok((stream, _)) = accept_result else {
            break;
        };
        let state = PoolServerState {
            preamble_count: state.preamble_count.clone(),
            connection_count: state.connection_count.clone(),
            serviced_connection_count: state.serviced_connection_count.clone(),
            special_response_handled: state.special_response_handled.clone(),
            delayed_first_response_started: state.delayed_first_response_started.clone(),
            delayed_first_response_release: state.delayed_first_response_release.clone(),
            delayed_first_response_attempted: state.delayed_first_response_attempted.clone(),
            behaviour: state.behaviour,
        };
        tokio::spawn(handle_pool_test_connection(stream, state));
    }
}

/// Perform one fixture handshake and echo length-delimited ping frames.
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
    // Count a connection once, when it first completes a real exchange. A
    // malformed reply is still an accepted connection, but it proves nothing
    // about the client having dispatched work here.
    let mut has_served_reply = false;
    loop {
        let frame = framed.next().await;
        let Some(Ok(bytes)) = frame else {
            break;
        };
        let decoded = bincode::decode_from_slice::<Ping, _>(&bytes, bincode::config::standard());
        let Ok((ping, _)) = decoded else {
            break;
        };

        let is_first_special_response =
            !state.special_response_handled.swap(true, Ordering::SeqCst);
        let Some(reply) =
            pool_test_response_payload(&ping, state.behaviour, is_first_special_response)
        else {
            break;
        };

        let is_late_first_response =
            matches!(state.behaviour, PoolServerBehavior::LateFirstResponse)
                && is_first_special_response;
        if !send_pool_test_response(&mut framed, reply.payload, is_late_first_response, &state)
            .await
        {
            break;
        }
        if reply.is_served && !has_served_reply {
            state
                .serviced_connection_count
                .fetch_add(1, Ordering::SeqCst);
            has_served_reply = true;
        }
    }
}

/// Encode a normal reply, or the empty payload used by malformed coverage.
///
/// This is the single place that decides whether a reply is well-formed, so
/// the `is_served` flag and the payload can never disagree about it.
fn pool_test_response_payload(
    ping: &Ping,
    behaviour: PoolServerBehavior,
    is_first_special_response: bool,
) -> Option<PoolTestReply> {
    let is_malformed = matches!(behaviour, PoolServerBehavior::MalformedFirstResponse)
        && is_first_special_response;
    let payload = if is_malformed {
        Vec::new()
    } else {
        bincode::encode_to_vec(Pong(ping.0), bincode::config::standard()).ok()?
    };

    Some(PoolTestReply {
        payload,
        is_served: !is_malformed,
    })
}

/// Reply payload for one request, and whether it proves a real exchange.
struct PoolTestReply {
    /// Bytes to write back to the client.
    payload: Vec<u8>,
    /// Whether the reply is a well-formed `Pong`; only those count as service.
    is_served: bool,
}

/// Send one reply, optionally holding it until the test releases it.
///
/// The signals let a cancellation test wait until the server has received the
/// request, cancel its client operation, and only then let the reply be
/// written into a socket the client has already abandoned.
///
/// Returns `false` when the connection can no longer be used.
async fn send_pool_test_response(
    framed: &mut Framed<tokio::net::TcpStream, LengthDelimitedCodec>,
    payload: Vec<u8>,
    is_late_first_response: bool,
    state: &PoolServerState,
) -> bool {
    if is_late_first_response {
        state.delayed_first_response_started.notify_one();
        state.delayed_first_response_release.notified().await;
    }

    // `Bytes::from`: 0.7.20 also encodes `&[u8]`, so `send` cannot infer.
    let send_result = framed.send(Bytes::from(payload)).await;
    if is_late_first_response {
        state.delayed_first_response_attempted.notify_one();
    }
    send_result.is_ok()
}

/// Decode the fixture's handshake response and return any buffered bytes.
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
