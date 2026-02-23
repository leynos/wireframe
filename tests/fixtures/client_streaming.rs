//! `ClientStreamingWorld` fixture for streaming response BDD tests.

mod modes;
mod server;
mod types;

use std::net::SocketAddr;

use futures::StreamExt;
use rstest::fixture;
use tokio::task::JoinHandle;
use wireframe::{
    client::{ClientError, WireframeClient},
    rewind_stream::RewindStream,
    serializer::{BincodeSerializer, Serializer},
};
pub use wireframe_testing::TestResult;

use self::types::{MessageId, Payload};
pub use self::{modes::*, types::*};

type _StreamingServerModeReexportAnchor = StreamingServerMode;

/// Test world for client streaming scenarios.
pub struct ClientStreamingWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>>,
    received_frames: Vec<StreamTestEnvelope>,
    last_error: Option<ClientError>,
    stream_terminated_cleanly: bool,
    shared_rate_limit_blocked: Option<bool>,
}

impl std::fmt::Debug for ClientStreamingWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientStreamingWorld")
            .field("addr", &self.addr)
            .field("received_frames", &self.received_frames.len())
            .field("stream_terminated_cleanly", &self.stream_terminated_cleanly)
            .finish_non_exhaustive()
    }
}

/// Fixture for `ClientStreamingWorld`.
#[rustfmt::skip]
#[fixture]
pub fn client_streaming_world() -> ClientStreamingWorld {
    ClientStreamingWorld::new()
}

impl ClientStreamingWorld {
    /// Build a new runtime-backed client streaming world.
    fn new() -> Self {
        let (runtime, runtime_error) = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => (Some(rt), None),
            Err(e) => (None, Some(format!("failed to create runtime: {e}"))),
        };
        Self {
            runtime,
            runtime_error,
            addr: None,
            server: None,
            client: None,
            received_frames: Vec::new(),
            last_error: None,
            stream_terminated_cleanly: false,
            shared_rate_limit_blocked: None,
        }
    }

    /// Run a future on the shared runtime, temporarily yielding ownership
    /// to avoid overlapping `&self` / `&mut self` borrows.
    pub fn block_on<T>(
        &mut self,
        f: impl for<'a> FnOnce(
            &'a mut Self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>,
    ) -> TestResult<T> {
        let err_msg = self
            .runtime_error
            .as_deref()
            .unwrap_or("runtime unavailable");
        let rt = self.runtime.take().ok_or(err_msg)?;
        let result = rt.block_on(f(self));
        self.runtime = Some(rt);
        Ok(result)
    }

    /// Connect a client to the server.
    pub async fn connect_client(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let client = WireframeClient::builder().connect(addr).await?;
        self.client = Some(client);
        Ok(())
    }

    /// Send a streaming request and consume the response stream.
    pub async fn send_streaming_request(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;

        let request = StreamTestEnvelope {
            id: MessageId::new(99),
            correlation_id: None,
            payload: Payload::new(vec![]),
        };

        let mut stream = client.call_streaming::<StreamTestEnvelope>(request).await?;

        self.received_frames.clear();
        self.last_error = None;
        self.stream_terminated_cleanly = false;
        self.shared_rate_limit_blocked = None;

        loop {
            match stream.next().await {
                Some(Ok(frame)) => {
                    self.received_frames.push(frame);
                }
                Some(Err(e)) => {
                    self.last_error = Some(e);
                    break;
                }
                None => {
                    self.stream_terminated_cleanly = true;
                    break;
                }
            }
        }

        Ok(())
    }
}
