//! `MessageAssemblyInboundWorld` fixture for inbound assembly integration.

use std::{fmt, future::Future, num::NonZeroUsize, time::Duration};

use bytes::{BufMut, BytesMut};
use futures::SinkExt;
use rstest::fixture;
use tokio::{io::DuplexStream, sync::mpsc, task::JoinHandle, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, Handler, WireframeApp},
    fragment::FragmentationConfig,
    serializer::BincodeSerializer,
    test_helpers::TestAssembler,
};
pub use wireframe_testing::TestResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageKey(pub u64);

impl From<u64> for MessageKey {
    fn from(value: u64) -> Self { Self(value) }
}

impl From<MessageKey> for u64 {
    fn from(value: MessageKey) -> Self { value.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameSequence(pub u32);

impl From<u32> for FrameSequence {
    fn from(value: u32) -> Self { Self(value) }
}

impl From<FrameSequence> for u32 {
    fn from(value: FrameSequence) -> Self { value.0 }
}

const ROUTE_ID: u32 = 77;
const CORRELATION_ID: Option<u64> = Some(5);
const BUFFER_CAPACITY: usize = 512;

/// Runtime-backed fixture that drives inbound message assembly through
/// `WireframeApp::handle_connection_result`.
pub struct MessageAssemblyInboundWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    client: Option<Framed<DuplexStream, LengthDelimitedCodec>>,
    server: Option<JoinHandle<std::io::Result<()>>>,
    observed_rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    observed_payloads: Vec<Vec<u8>>,
    last_send_error: Option<String>,
}

impl fmt::Debug for MessageAssemblyInboundWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageAssemblyInboundWorld")
            .field("client_initialized", &self.client.is_some())
            .field("server_initialized", &self.server.is_some())
            .field("observed_payloads", &self.observed_payloads.len())
            .field("last_send_error", &self.last_send_error)
            .finish_non_exhaustive()
    }
}

impl Default for MessageAssemblyInboundWorld {
    fn default() -> Self {
        match tokio::runtime::Runtime::new() {
            Ok(runtime) => Self {
                runtime: Some(runtime),
                runtime_error: None,
                client: None,
                server: None,
                observed_rx: None,
                observed_payloads: Vec::new(),
                last_send_error: None,
            },
            Err(err) => Self {
                runtime: None,
                runtime_error: Some(format!("failed to create runtime: {err}")),
                client: None,
                server: None,
                observed_rx: None,
                observed_payloads: Vec::new(),
                last_send_error: None,
            },
        }
    }
}

// rustfmt collapses simple fixtures into one line, which triggers unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn message_assembly_inbound_world() -> MessageAssemblyInboundWorld {
    MessageAssemblyInboundWorld::default()
}

impl MessageAssemblyInboundWorld {
    fn runtime(&self) -> TestResult<&tokio::runtime::Runtime> {
        self.runtime.as_ref().ok_or_else(|| {
            self.runtime_error
                .clone()
                .unwrap_or_else(|| "runtime unavailable".to_string())
                .into()
        })
    }

    fn block_on<F, T>(&self, future: F) -> TestResult<T>
    where
        F: Future<Output = T>,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("nested Tokio runtime detected in inbound assembly fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    pub fn start_app(&mut self, timeout_ms: u64) -> TestResult {
        let message_limit =
            NonZeroUsize::new(BUFFER_CAPACITY.saturating_mul(16)).unwrap_or(NonZeroUsize::MIN);
        let config = FragmentationConfig::for_frame_budget(
            BUFFER_CAPACITY,
            message_limit,
            Duration::from_millis(timeout_ms),
        )
        .ok_or("frame budget too small for fragmentation config")?;

        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let handler: Handler<Envelope> = std::sync::Arc::new(move |env: &Envelope| {
            let tx = tx.clone();
            let payload = env.payload_bytes().to_vec();
            Box::pin(async move {
                let _ = tx.send(payload);
            })
        });

        let app: WireframeApp = WireframeApp::new()?
            .buffer_capacity(BUFFER_CAPACITY)
            .fragmentation(Some(config))
            .with_message_assembler(TestAssembler)
            .route(ROUTE_ID, handler)?;

        let codec = app.length_codec();
        let (client_stream, server_stream) = tokio::io::duplex(2048);
        let client = Framed::new(client_stream, codec);
        let server = self
            .runtime()?
            .spawn(async move { app.handle_connection_result(server_stream).await });

        self.client = Some(client);
        self.server = Some(server);
        self.observed_rx = Some(rx);
        self.observed_payloads.clear();
        self.last_send_error = None;
        Ok(())
    }

    pub fn send_first_frame(&mut self, key: impl Into<MessageKey>, body: &str) -> TestResult {
        let key = key.into();
        self.send_payload(first_frame_payload(key.0, body.as_bytes(), false, None))
    }

    pub fn send_continuation_frame(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
    ) -> TestResult {
        let key = key.into();
        let sequence = sequence.into();
        self.send_continuation_frame_impl(key.0, sequence.0, body, false)
    }

    pub fn send_final_continuation_frame(
        &mut self,
        key: impl Into<MessageKey>,
        sequence: impl Into<FrameSequence>,
        body: &str,
    ) -> TestResult {
        let key = key.into();
        let sequence = sequence.into();
        self.send_continuation_frame_impl(key.0, sequence.0, body, true)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "helper signature is explicitly required by the inbound assembly refactor task"
    )]
    fn send_continuation_frame_impl(
        &mut self,
        key: u64,
        sequence: u32,
        body: &str,
        is_last: bool,
    ) -> TestResult {
        self.send_payload(continuation_frame_payload(
            key,
            sequence,
            body.as_bytes(),
            is_last,
        ))
    }

    pub fn wait_millis(&mut self, millis: u64) -> TestResult {
        self.block_on(async { tokio::time::sleep(Duration::from_millis(millis)).await })?;
        Ok(())
    }

    pub fn assert_received_payload(&mut self, expected: &str) -> TestResult {
        self.collect_observed_for(Duration::from_millis(200))?;
        let expected = expected.as_bytes();
        if self
            .observed_payloads
            .iter()
            .any(|payload| payload.as_slice() == expected)
        {
            return Ok(());
        }

        Err(format!(
            "expected payload {:?} not observed; observed={:?}",
            expected, self.observed_payloads
        )
        .into())
    }

    pub fn assert_received_count(&mut self, expected_count: usize) -> TestResult {
        self.collect_observed_for(Duration::from_millis(120))?;
        let actual = self.observed_payloads.len();
        if actual == expected_count {
            return Ok(());
        }
        Err(format!("expected {expected_count} payloads, got {actual}").into())
    }

    pub fn assert_no_send_error(&self) -> TestResult {
        if self.last_send_error.is_none() {
            return Ok(());
        }
        Err(format!("unexpected send error: {:?}", self.last_send_error).into())
    }

    fn send_payload(&mut self, payload: Vec<u8>) -> TestResult {
        let mut client = self.client.take().ok_or("client not initialized")?;
        let envelope = Envelope::new(ROUTE_ID, CORRELATION_ID, payload);
        let serializer = BincodeSerializer;
        let frame = serializer.serialize(&envelope)?;

        let send_result = self.block_on(async {
            client.send(frame.into()).await?;
            client.flush().await?;
            Ok::<(), std::io::Error>(())
        });

        self.client = Some(client);
        match send_result {
            Ok(Ok(())) => {
                self.last_send_error = None;
                Ok(())
            }
            Ok(Err(err)) => {
                self.last_send_error = Some(err.to_string());
                Err(err.into())
            }
            Err(err) => {
                self.last_send_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    fn collect_observed_for(&mut self, max_wait: Duration) -> TestResult {
        let mut observed_rx = self.observed_rx.take().ok_or("receiver not initialized")?;
        let collected = self.block_on(async {
            let mut collected = Vec::new();
            while let Ok(Some(payload)) = timeout(max_wait, observed_rx.recv()).await {
                collected.push(payload);
            }
            collected
        })?;
        self.observed_payloads.extend(collected);
        self.observed_rx = Some(observed_rx);
        Ok(())
    }
}

fn first_frame_payload(key: u64, body: &[u8], is_last: bool, total: Option<u32>) -> Vec<u8> {
    let mut payload = BytesMut::new();
    payload.put_u8(0x01);
    let mut flags = 0u8;
    if is_last {
        flags |= 0b1;
    }
    if total.is_some() {
        flags |= 0b10;
    }
    payload.put_u8(flags);
    payload.put_u64(key);
    payload.put_u16(0);
    payload.put_u32(u32::try_from(body.len()).unwrap_or(u32::MAX));
    if let Some(total) = total {
        payload.put_u32(total);
    }
    payload.extend_from_slice(body);
    payload.to_vec()
}

fn continuation_frame_payload(key: u64, sequence: u32, body: &[u8], is_last: bool) -> Vec<u8> {
    let mut payload = BytesMut::new();
    payload.put_u8(0x02);
    let mut flags = 0b10;
    if is_last {
        flags |= 0b1;
    }
    payload.put_u8(flags);
    payload.put_u64(key);
    payload.put_u32(u32::try_from(body.len()).unwrap_or(u32::MAX));
    payload.put_u32(sequence);
    payload.extend_from_slice(body);
    payload.to_vec()
}
