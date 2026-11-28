//! `ClientTracingWorld` fixture for rstest-bdd tracing tests.
//!
//! Provides a thread-local tracing subscriber that captures formatted output
//! into a shared buffer for assertion in step definitions.

#![expect(
    clippy::excessive_nesting,
    reason = "async closures within spawned echo server are inherently nested"
)]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::fixture;
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::Envelope,
    client::{TracingConfig, WireframeClient},
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
    test_helpers::ServerTaskGuard,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;
use wireframe_testing::{ServerMode, process_frame};

/// Client type alias for tracing tests.
type TestClient = WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>;

/// Writer that appends formatted tracing output to a shared buffer.
#[derive(Clone, Debug)]
struct CaptureWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl CaptureWriter {
    fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self { Self { buf } }
}

impl std::io::Write for CaptureWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer { self.clone() }
}

/// Test world for client tracing BDD scenarios.
pub struct ClientTracingWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    addr: Option<SocketAddr>,
    server: Option<ServerTaskGuard<()>>,
    client: Option<TestClient>,
    tracing_config: TracingConfig,
    captured: Arc<Mutex<Vec<u8>>>,
    _subscriber_guard: tracing::subscriber::DefaultGuard,
}

impl Drop for ClientTracingWorld {
    /// Stop the echo server and await its termination.
    ///
    /// `Drop` cannot report a panicking server, so the abort-and-join outcome
    /// is discarded; the await still guarantees the task is reaped before the
    /// runtime it belongs to goes away. Without a runtime there is nothing to
    /// drive the await on, and the guard's abort-on-drop fallback stops the
    /// task instead.
    fn drop(&mut self) {
        let Some(server) = self.server.take() else {
            return;
        };
        // Without a runtime there is nothing to drive the await on, so
        // returning here drops the guard and its abort-on-drop fallback stops
        // the task.
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let _cleanup = runtime.block_on(server.abort_and_join());
    }
}

/// Create a new `ClientTracingWorld` with default tracing config.
fn new_world(config: TracingConfig) -> ClientTracingWorld {
    let (runtime, runtime_error) = match tokio::runtime::Runtime::new() {
        Ok(rt) => (Some(rt), None),
        Err(err) => (None, Some(format!("failed to create runtime: {err}"))),
    };
    let captured = Arc::new(Mutex::new(Vec::new()));
    let writer = CaptureWriter::new(Arc::clone(&captured));

    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_level(true)
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    ClientTracingWorld {
        runtime,
        runtime_error,
        addr: None,
        server: None,
        client: None,
        tracing_config: config,
        captured,
        _subscriber_guard: guard,
    }
}

/// Fixture for `ClientTracingWorld` with default tracing config.
#[rustfmt::skip]
#[fixture]
pub fn client_tracing_world() -> ClientTracingWorld {
    new_world(TracingConfig::default())
}

impl ClientTracingWorld {
    /// Start a standard echo server.
    ///
    /// Any server this world already owns is stopped and awaited first, so a
    /// second start never abandons a running task. That server's panic is
    /// reported instead of being discarded.
    ///
    /// # Errors
    /// Returns an error if the previous server panicked, or if binding fails.
    pub async fn start_echo_server(&mut self) -> TestResult {
        self.reap_existing_server().await?;

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

            while let Some(Ok(bytes)) = framed.next().await {
                let Some(response_bytes) = process_frame(ServerMode::Echo, &bytes) else {
                    break;
                };
                if framed.send(Bytes::from(response_bytes)).await.is_err() {
                    break;
                }
            }
        });

        self.addr = Some(addr);
        self.server = Some(ServerTaskGuard::new(handle));
        Ok(())
    }

    /// Stop and await the server this world currently owns, if any.
    ///
    /// # Errors
    /// Returns an error if that server panicked.
    async fn reap_existing_server(&mut self) -> TestResult {
        let Some(previous) = self.server.take() else {
            return Ok(());
        };
        previous.abort_and_join().await?;
        Ok(())
    }

    /// Update the tracing config.
    pub fn set_tracing_config(&mut self, config: TracingConfig) { self.tracing_config = config; }

    /// Return a cloned handle to the owned Tokio runtime.
    ///
    /// Callers use `let rt = world.handle()?; rt.block_on(world.some_async_method())`
    /// to run async methods: cloning the handle first avoids conflicting
    /// borrows between the runtime and the `&mut self` taken by async methods.
    ///
    /// # Errors
    /// Returns an error if the runtime failed to initialize.
    pub fn handle(&self) -> TestResult<tokio::runtime::Handle> {
        self.runtime
            .as_ref()
            .map(|rt| rt.handle().clone())
            .ok_or_else(|| {
                self.runtime_error
                    .clone()
                    .unwrap_or_else(|| "runtime unavailable".to_owned())
                    .into()
            })
    }

    /// Connect to the server with the current tracing config.
    ///
    /// # Errors
    /// Returns an error if connection fails.
    pub async fn connect(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let client = WireframeClient::builder()
            .tracing_config(self.tracing_config.clone())
            .connect(addr)
            .await?;
        self.client = Some(client);
        Ok(())
    }

    /// Send an envelope via the connected client.
    ///
    /// # Errors
    /// Returns an error if send fails.
    pub async fn send_envelope(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        client.send(&envelope).await?;
        Ok(())
    }

    /// Send an envelope and receive the echoed response.
    ///
    /// # Errors
    /// Returns an error if send or receive fails.
    pub async fn send_and_receive(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        let _response: Envelope = client.call(&envelope).await?;
        Ok(())
    }

    /// Close the client connection.
    pub async fn close_connection(&mut self) {
        if let Some(client) = self.client.take() {
            client.close().await;
        }
    }

    /// Return the peer address string.
    pub fn peer_addr_string(&self) -> String {
        self.addr.map_or_else(String::new, |a| a.to_string())
    }

    /// Check whether captured tracing output contains a needle.
    pub fn output_contains(&self, needle: &str) -> bool {
        let Ok(buf) = self.captured.lock() else {
            return false;
        };
        let output = String::from_utf8_lossy(&buf);
        output.contains(needle)
    }

    /// Assert that the output contains the needle.
    ///
    /// # Errors
    /// Returns an error if the needle is not found.
    pub fn assert_output_contains(&self, needle: &str) -> TestResult {
        if self.output_contains(needle) {
            Ok(())
        } else {
            let buf = self
                .captured
                .lock()
                .map_err(|e| std::io::Error::other(format!("lock captured buffer: {e}")))?;
            let output = String::from_utf8_lossy(&buf);
            Err(format!("expected output to contain {needle:?}, got:\n{output}").into())
        }
    }

    /// Assert that the output does NOT contain the needle.
    ///
    /// # Errors
    /// Returns an error if the needle is found.
    pub fn assert_output_not_contains(&self, needle: &str) -> TestResult {
        if self.output_contains(needle) {
            Err(format!("expected output NOT to contain {needle:?}, but it was present").into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    //! Regression tests for scenario teardown.

    use super::*;
    use crate::fixtures::teardown_probe::{
        assert_teardown_awaits,
        assert_waited_for_cancellation,
        spawn_panicking_task,
        spawn_slow_cancel_task,
    };

    /// Teardown must await the abort, not merely request one.
    ///
    /// The probe task runs on a runtime that outlives the world, so only an
    /// awaited abort can make teardown block for the cancellation delay.
    #[test]
    fn drop_awaits_the_server_abort() {
        let probe_runtime = tokio::runtime::Runtime::new().expect("probe runtime");
        let mut world = client_tracing_world();
        world.server = Some(ServerTaskGuard::new(spawn_slow_cancel_task(
            probe_runtime.handle(),
        )));

        assert_teardown_awaits(|| drop(world));
        drop(probe_runtime);
    }

    /// Starting the echo server twice must leave only the second one running.
    ///
    /// Unlike the probe test, this drives the real server: the first listener
    /// must be closed and its task gone by the time the second start returns.
    #[test]
    fn starting_twice_replaces_the_first_server() {
        let mut world = client_tracing_world();
        let runtime = world.handle().expect("runtime");
        let baseline = runtime.metrics().num_alive_tasks();

        runtime
            .block_on(world.start_echo_server())
            .expect("first start");
        let first_addr = world.addr.expect("first server address");
        runtime
            .block_on(world.start_echo_server())
            .expect("second start");

        let first_still_listening = runtime
            .block_on(tokio::net::TcpStream::connect(first_addr))
            .is_ok();
        assert!(
            !first_still_listening,
            "the first server must be reaped before the second starts"
        );
        assert_eq!(
            runtime.metrics().num_alive_tasks(),
            baseline + 1,
            "only the second server may remain alive"
        );
    }

    /// Starting a second server must reap the first, not abandon it.
    #[test]
    fn start_echo_server_reaps_the_previous_server() {
        let probe_runtime = tokio::runtime::Runtime::new().expect("probe runtime");
        let mut world = client_tracing_world();
        world.server = Some(ServerTaskGuard::new(spawn_slow_cancel_task(
            probe_runtime.handle(),
        )));

        let runtime = world.handle().expect("runtime");
        let started = std::time::Instant::now();
        let outcome = runtime.block_on(world.start_echo_server());
        let elapsed = started.elapsed();

        assert!(
            outcome.is_ok(),
            "starting a second server must succeed: {outcome:?}"
        );
        assert_waited_for_cancellation(elapsed);
        drop(probe_runtime);
    }

    /// A panicking previous server must be reported, not replaced silently.
    #[test]
    fn start_echo_server_reports_a_panicking_previous_server() {
        let mut world = client_tracing_world();
        let runtime = world.handle().expect("runtime");
        world.server = Some(ServerTaskGuard::new(spawn_panicking_task(&runtime)));

        let outcome = runtime.block_on(world.start_echo_server());

        assert!(
            outcome.is_err(),
            "a panicking previous server must surface as a step error"
        );
    }
}
