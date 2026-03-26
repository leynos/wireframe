//! `ClientStreamingWorld` fixture for streaming response BDD tests.

mod modes;
mod server;
mod types;

use std::net::SocketAddr;

use futures::StreamExt;
use rstest::fixture;
use tokio::task::JoinHandle;
use wireframe::{
    client::{ClientError, StreamingResponseExt, WireframeClient},
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
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
    typed_items: Vec<TypedStreamingItem>,
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
    fn build_request() -> StreamTestEnvelope {
        StreamTestEnvelope {
            id: MessageId::new(99),
            correlation_id: None,
            payload: Payload::new(vec![]),
        }
    }

    fn reset_state(&mut self) {
        self.received_frames.clear();
        self.typed_items.clear();
        self.last_error = None;
        self.stream_terminated_cleanly = false;
        self.shared_rate_limit_blocked = None;
    }

    async fn drain_stream<St, Item, F>(&mut self, mut stream: St, mut push: F)
    where
        St: futures::Stream<Item = Result<Item, ClientError>> + Unpin,
        F: FnMut(&mut Self, Item),
    {
        loop {
            match stream.next().await {
                Some(Ok(item)) => push(self, item),
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
    }

    async fn execute_stream_call<Item>(
        &mut self,
        collect: impl for<'a> FnOnce(
            &'a mut WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<Vec<Result<Item, ClientError>>, ClientError>,
                    > + 'a,
            >,
        >,
        push: impl FnMut(&mut Self, Item),
    ) -> TestResult {
        self.reset_state();
        let mut client = self.client.take().ok_or("client not connected")?;
        let items = collect(&mut client).await?;
        self.client = Some(client);
        self.drain_stream(futures::stream::iter(items), push).await;
        Ok(())
    }

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
            typed_items: Vec::new(),
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
        self.execute_stream_call(
            |client| {
                Box::pin(async move {
                    let stream = client
                        .call_streaming::<StreamTestEnvelope>(Self::build_request())
                        .await?;
                    Ok(stream.collect().await)
                })
            },
            |world, frame| world.received_frames.push(frame),
        )
        .await
    }

    /// Send a streaming request and collect only typed data items.
    pub async fn send_typed_streaming_request(&mut self) -> TestResult {
        self.execute_stream_call(
            |client| {
                Box::pin(async move {
                    let stream = client
                        .call_streaming::<StreamTestEnvelope>(Self::build_request())
                        .await?
                        .typed_with(map_streaming_item);
                    Ok(stream.collect().await)
                })
            },
            |world, item| world.typed_items.push(item),
        )
        .await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedStreamingItem(u8);

impl TypedStreamingItem {
    pub const fn value(&self) -> u8 { self.0 }
}

fn map_streaming_item(
    frame: StreamTestEnvelope,
) -> Result<Option<TypedStreamingItem>, ClientError> {
    match frame.id.get() {
        1 | 2 | 3 | 4 | 10 | 11 => {
            let value = frame
                .payload
                .into_inner()
                .into_iter()
                .next()
                .ok_or_else(|| {
                    ClientError::from(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "missing payload byte",
                    ))
                })?;
            Ok(Some(TypedStreamingItem(value)))
        }
        200 | 201 | 250 => Ok(None),
        other => Err(ClientError::from(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unexpected frame id {other}"),
        ))),
    }
}
