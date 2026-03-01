//! `ClientSendStreamingWorld` fixture for outbound streaming send BDD tests.

use std::{io, net::SocketAddr, time::Duration};

use futures::StreamExt;
use rstest::fixture;
use tokio::{io::AsyncWriteExt, net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::client::{ClientError, SendStreamingConfig, WireframeClient};
pub use wireframe_testing::TestResult;

/// Test world for outbound streaming send scenarios.
pub struct ClientSendStreamingWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<Vec<Vec<u8>>>>,
    client: Option<
        WireframeClient<
            wireframe::serializer::BincodeSerializer,
            wireframe::rewind_stream::RewindStream<tokio::net::TcpStream>,
        >,
    >,
    received_frames: Vec<Vec<u8>>,
    frames_sent: Option<u64>,
    last_error: Option<ClientError>,
    protocol_header: Vec<u8>,
}

impl std::fmt::Debug for ClientSendStreamingWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientSendStreamingWorld")
            .field("addr", &self.addr)
            .field("frames_sent", &self.frames_sent)
            .finish_non_exhaustive()
    }
}

/// Fixture for `ClientSendStreamingWorld`.
#[rustfmt::skip]
#[fixture]
pub fn client_send_streaming_world() -> ClientSendStreamingWorld {
    ClientSendStreamingWorld::new()
}

/// Generate a body of `n` bytes for testing: cycles through 0–255.
///
/// This mirrors the identically named helper in
/// `src/client/tests/send_streaming_infra.rs`. The two cannot be shared
/// directly because that helper is `pub(super)` inside the library
/// crate.
#[expect(
    clippy::integer_division_remainder_used,
    reason = "modulo generates a deterministic test byte pattern"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "value is modulo 256, guaranteed to fit in u8"
)]
fn test_body(n: usize) -> Vec<u8> { (0..n).map(|i| (i % 256) as u8).collect() }

impl ClientSendStreamingWorld {
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
            frames_sent: None,
            last_error: None,
            protocol_header: vec![0xca, 0xfe, 0xba, 0xbe],
        }
    }

    /// Run a future on the shared runtime.
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

    /// Start a receiving server that collects all frames sent by the client.
    ///
    /// The server task returns the collected frames when the client
    /// disconnects. Call [`collect_server_frames`] after the send to
    /// retrieve them.
    pub async fn start_receiving_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        self.addr = Some(listener.local_addr()?);

        self.server = Some(tokio::spawn(async move {
            let mut collected = Vec::new();
            let Ok((tcp, _)) = listener.accept().await else {
                return collected;
            };
            let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());
            while let Some(Ok(bytes)) = transport.next().await {
                collected.push(bytes.to_vec());
            }
            collected
        }));

        Ok(())
    }

    /// Start a server that accepts and immediately shuts down the write
    /// side, triggering a deterministic transport error on the client.
    pub async fn start_dropping_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        self.addr = Some(listener.local_addr()?);

        // Spawn as a `JoinHandle<Vec<Vec<u8>>>` so the type matches.
        self.server = Some(tokio::spawn(async move {
            let Ok((mut tcp, _)) = listener.accept().await else {
                return Vec::new();
            };
            let _ = tcp.shutdown().await;
            Vec::new()
        }));

        Ok(())
    }

    /// Connect a client to the server.
    pub async fn connect_client(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let client = WireframeClient::builder().connect(addr).await?;
        self.client = Some(client);
        Ok(())
    }

    /// Abort the existing server handle.
    pub fn abort_server(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }

    /// Perform a streaming send with the given parameters.
    ///
    /// `header_size` controls the length of the protocol header used for
    /// this send — the header is resized (truncated or zero-padded) to
    /// match the requested size.
    pub async fn do_send_streaming(
        &mut self,
        body_size: usize,
        header_size: usize,
        chunk_size: usize,
    ) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let mut header = self.protocol_header.clone();
        header.resize(header_size, 0x00);
        let body = test_body(body_size);
        let config = SendStreamingConfig::default().with_chunk_size(chunk_size);

        match client.send_streaming(&header, &body[..], config).await {
            Ok(outcome) => {
                self.frames_sent = Some(outcome.frames_sent());
            }
            Err(e) => {
                self.last_error = Some(e);
            }
        }
        Ok(())
    }

    /// Perform a streaming send with a timeout against a blocking reader.
    pub async fn do_send_streaming_with_timeout(&mut self, timeout: Duration) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let header = self.protocol_header.clone();

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, io::Error>>(1);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let reader = tokio_util::io::StreamReader::new(stream);

        let config = SendStreamingConfig::default()
            .with_chunk_size(100)
            .with_timeout(timeout);

        let result = client.send_streaming(&header, reader, config).await;
        drop(tx);

        match result {
            Ok(outcome) => {
                self.frames_sent = Some(outcome.frames_sent());
            }
            Err(e) => {
                self.last_error = Some(e);
            }
        }
        Ok(())
    }

    /// Drop the client and await the server task to collect received frames.
    pub async fn collect_server_frames(&mut self) -> TestResult {
        // Drop the client so the server sees EOF.
        self.client.take();
        if let Some(handle) = self.server.take() {
            self.received_frames = handle
                .await
                .map_err(|e| format!("server task panicked: {e}"))?;
        }
        Ok(())
    }

    /// Verify the number of frames received by the server.
    pub fn verify_server_frame_count(&self, expected: usize) -> TestResult {
        if self.received_frames.len() != expected {
            return Err(format!(
                "expected {expected} frames, got {}",
                self.received_frames.len()
            )
            .into());
        }
        Ok(())
    }

    /// Verify that each received frame starts with the protocol header.
    pub fn verify_frames_start_with_header(&self) -> TestResult {
        for (i, frame) in self.received_frames.iter().enumerate() {
            if !frame.starts_with(&self.protocol_header) {
                return Err(format!("frame {i} does not start with the protocol header").into());
            }
        }
        Ok(())
    }

    /// Verify that the last error is a `TimedOut` I/O error.
    pub fn verify_timed_out_error(&self) -> TestResult {
        let err = self
            .last_error
            .as_ref()
            .ok_or("expected an error but none occurred")?;
        match err {
            ClientError::Wireframe(wireframe::WireframeError::Io(io_err)) => {
                if io_err.kind() != io::ErrorKind::TimedOut {
                    return Err(format!("expected TimedOut, got {:?}", io_err.kind()).into());
                }
                Ok(())
            }
            other => Err(format!("expected Wireframe(Io(TimedOut)), got {other:?}").into()),
        }
    }

    /// Verify that the last error is a transport error.
    pub fn verify_transport_error(&self) -> TestResult {
        let err = self
            .last_error
            .as_ref()
            .ok_or("expected an error but none occurred")?;
        match err {
            ClientError::Wireframe(wireframe::WireframeError::Io(_)) => Ok(()),
            other => Err(format!("expected Wireframe(Io(_)), got {other:?}").into()),
        }
    }
}
