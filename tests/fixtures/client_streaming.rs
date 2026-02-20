//! `ClientStreamingWorld` fixture for streaming response BDD tests.

mod server;
mod types;

use std::net::SocketAddr;

use futures::StreamExt;
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    WireframeError,
    client::{ClientError, WireframeClient},
    correlation::CorrelatableFrame,
    rewind_stream::RewindStream,
    serializer::{BincodeSerializer, Serializer},
};
pub use wireframe_testing::TestResult;

pub use self::types::*;
use self::{
    server::{send_data_and_terminator, send_data_frames, send_mismatch_frame},
    types::{CorrelationId, MessageId, Payload},
};

/// Mode controlling how the streaming test server behaves.
enum StreamingServerMode {
    /// Send `data_count` data frames then a terminator.
    Normal { data_count: usize },
    /// Send one frame with a wrong correlation ID.
    Mismatch,
    /// Send `data_count` data frames then drop the connection.
    Disconnect { data_count: usize },
}

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
        let (runtime, runtime_error) = match tokio::runtime::Runtime::new() {
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

    /// Start a streaming server that sends `data_count` frames + terminator.
    pub async fn start_normal_server(&mut self, data_count: usize) -> TestResult {
        self.start_server(StreamingServerMode::Normal { data_count })
            .await
    }

    /// Start a streaming server that returns a mismatched correlation ID.
    pub async fn start_mismatch_server(&mut self) -> TestResult {
        self.start_server(StreamingServerMode::Mismatch).await
    }

    /// Start a server that sends `data_count` frames then disconnects.
    pub async fn start_disconnect_server(&mut self, data_count: usize) -> TestResult {
        self.start_server(StreamingServerMode::Disconnect { data_count })
            .await
    }

    async fn start_server(&mut self, mode: StreamingServerMode) -> TestResult {
        self.abort_server();

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

            // Read the client's request to extract the correlation ID.
            let Some(Ok(req_bytes)) = framed.next().await else {
                return;
            };
            let Ok((req, _)): Result<(StreamTestEnvelope, usize), _> =
                BincodeSerializer.deserialize(&req_bytes)
            else {
                return;
            };
            let cid = CorrelationId::new(req.correlation_id().unwrap_or(1));

            match mode {
                StreamingServerMode::Normal { data_count } => {
                    send_data_and_terminator(&mut framed, cid, data_count).await;
                }
                StreamingServerMode::Mismatch => {
                    send_mismatch_frame(&mut framed, cid).await;
                }
                StreamingServerMode::Disconnect { data_count } => {
                    send_data_frames(&mut framed, cid, data_count).await;
                    // Drop without terminator.
                }
            }
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
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

    /// Verify the count of received data frames.
    pub fn verify_frame_count(&self, expected: usize) -> TestResult {
        let actual = self.received_frames.len();
        if actual != expected {
            return Err(format!("expected {expected} frames, got {actual}").into());
        }
        Ok(())
    }

    /// Verify frames arrived in order (payload == [1], [2], ...).
    pub fn verify_frame_order(&self) -> TestResult {
        for (i, frame) in self.received_frames.iter().enumerate() {
            let payload_byte =
                u8::try_from(i + 1).map_err(|e| format!("frame index {i} overflows u8: {e}"))?;
            let expected = Payload::new(vec![payload_byte]);
            if frame.payload != expected {
                return Err(format!(
                    "frame {i}: expected payload {expected:?}, got {:?}",
                    frame.payload
                )
                .into());
            }
        }
        Ok(())
    }

    /// Verify the stream terminated cleanly (received `None`).
    pub fn verify_clean_termination(&self) -> TestResult {
        if !self.stream_terminated_cleanly {
            return Err("stream did not terminate cleanly".into());
        }
        Ok(())
    }

    /// Verify that a `StreamCorrelationMismatch` error was returned.
    pub fn verify_correlation_mismatch_error(&self) -> TestResult {
        match &self.last_error {
            Some(ClientError::StreamCorrelationMismatch { .. }) => Ok(()),
            Some(err) => Err(format!("expected StreamCorrelationMismatch, got {err:?}").into()),
            None => Err("expected StreamCorrelationMismatch, but no error".into()),
        }
    }

    /// Verify that a transport/disconnect error was returned.
    pub fn verify_disconnect_error(&self) -> TestResult {
        match &self.last_error {
            Some(ClientError::Wireframe(WireframeError::Io(_))) => Ok(()),
            Some(err) => Err(format!("expected transport error, got {err:?}").into()),
            None => Err("expected transport error, but no error".into()),
        }
    }

    /// Abort the server task.
    pub fn abort_server(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}
