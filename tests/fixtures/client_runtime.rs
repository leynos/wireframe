//! `ClientRuntimeWorld` fixture for rstest-bdd tests.
//!
//! Provides an echo server/client pair to validate client runtime framing
//! behaviour.

use std::{
    cell::{Cell, RefCell},
    net::SocketAddr,
};

use futures::{SinkExt, StreamExt};
use log::warn;
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    BincodeSerializer,
    client::{ClientCodecConfig, ClientError, WireframeClient},
    rewind_stream::RewindStream,
};

/// `TestResult` for step definitions.
pub use crate::common::TestResult;

/// Test world exercising the wireframe client runtime.
#[derive(Debug)]
pub struct ClientRuntimeWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    addr: Cell<Option<SocketAddr>>,
    server: RefCell<Option<JoinHandle<()>>>,
    client:
        RefCell<Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>>>,
    payload: RefCell<Option<ClientPayload>>,
    response: RefCell<Option<ClientPayload>>,
    last_error: RefCell<Option<ClientError>>,
}

impl ClientRuntimeWorld {
    /// Build a new runtime-backed client world.
    pub fn new() -> Self {
        match tokio::runtime::Runtime::new() {
            Ok(runtime) => Self {
                runtime: Some(runtime),
                runtime_error: None,
                addr: Cell::new(None),
                server: RefCell::new(None),
                client: RefCell::new(None),
                payload: RefCell::new(None),
                response: RefCell::new(None),
                last_error: RefCell::new(None),
            },
            Err(err) => Self {
                runtime: None,
                runtime_error: Some(format!("failed to create runtime: {err}")),
                addr: Cell::new(None),
                server: RefCell::new(None),
                client: RefCell::new(None),
                payload: RefCell::new(None),
                response: RefCell::new(None),
                last_error: RefCell::new(None),
            },
        }
    }

    fn runtime(&self) -> TestResult<&tokio::runtime::Runtime> {
        self.runtime.as_ref().ok_or_else(|| {
            self.runtime_error
                .clone()
                .unwrap_or_else(|| "runtime unavailable".to_string())
                .into()
        })
    }
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq, Clone)]
struct ClientPayload {
    data: Vec<u8>,
}

/// Fixture for `ClientRuntimeWorld`.
#[fixture]
pub fn client_runtime_world() -> ClientRuntimeWorld {
    let world = ClientRuntimeWorld::new();
    let _ = world.runtime_error.as_deref();
    world
}

impl ClientRuntimeWorld {
    /// Start an echo server with the specified maximum frame length.
    ///
    /// # Errors
    /// Returns an error if binding or spawning the server fails.
    pub fn start_server(&self, max_frame_length: usize) -> TestResult {
        let runtime = self.runtime()?;
        let listener = runtime.block_on(async { TcpListener::bind("127.0.0.1:0").await })?;
        let addr = listener.local_addr()?;
        let handle = runtime.spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                warn!("client runtime server failed to accept connection");
                return;
            };
            let codec = LengthDelimitedCodec::builder()
                .max_frame_length(max_frame_length)
                .new_codec();
            let mut framed = Framed::new(stream, codec);
            let Some(result) = framed.next().await else {
                warn!("client runtime server closed before receiving a frame");
                return;
            };
            let Ok(frame) = result else {
                warn!("client runtime server failed to decode frame");
                return;
            };
            if let Err(err) = framed.send(frame.freeze()).await {
                warn!("client runtime server failed to send response: {err:?}");
            }
        });

        self.addr.set(Some(addr));
        *self.server.borrow_mut() = Some(handle);
        Ok(())
    }

    /// Connect a client using the specified maximum frame length.
    ///
    /// # Errors
    /// Returns an error if the server has not started or the client fails to connect.
    pub fn connect_client(&self, max_frame_length: usize) -> TestResult {
        let addr = self.addr.get().ok_or("server address missing")?;
        let codec_config = ClientCodecConfig::default().max_frame_length(max_frame_length);
        let runtime = self.runtime()?;
        let client = runtime.block_on(async {
            WireframeClient::builder()
                .codec_config(codec_config)
                .connect(addr)
                .await
        })?;
        *self.client.borrow_mut() = Some(client);
        Ok(())
    }

    /// Send a payload of the specified size and capture the response.
    ///
    /// # Errors
    /// Returns an error if the client is missing or communication fails.
    pub fn send_payload(&self, size: usize) -> TestResult {
        let payload = ClientPayload {
            data: vec![7_u8; size],
        };
        let mut client = self
            .client
            .borrow_mut()
            .take()
            .ok_or("client not connected")?;
        let runtime = self.runtime()?;
        let response: ClientPayload = runtime.block_on(async { client.call(&payload).await })?;
        *self.client.borrow_mut() = Some(client);
        *self.payload.borrow_mut() = Some(payload);
        *self.response.borrow_mut() = Some(response);
        *self.last_error.borrow_mut() = None;
        Ok(())
    }

    /// Send a payload that should exceed the peer's frame limit.
    ///
    /// # Errors
    /// Returns an error if the client is missing or if no failure is observed.
    pub fn send_payload_expect_error(&self, size: usize) -> TestResult {
        let payload = ClientPayload {
            data: vec![7_u8; size],
        };
        let mut client = self
            .client
            .borrow_mut()
            .take()
            .ok_or("client not connected")?;
        let runtime = self.runtime()?;
        let result: Result<ClientPayload, ClientError> =
            runtime.block_on(async { client.call(&payload).await });
        *self.client.borrow_mut() = Some(client);
        match result {
            Ok(_) => return Err("expected client error for oversized payload".into()),
            Err(err) => *self.last_error.borrow_mut() = Some(err),
        }
        Ok(())
    }

    /// Verify that the client received the echoed payload.
    ///
    /// # Errors
    /// Returns an error if the response is missing or mismatched.
    pub fn verify_echo(&self) -> TestResult {
        let payload_ref = self.payload.borrow();
        let response_ref = self.response.borrow();
        let payload = payload_ref.as_ref().ok_or("payload missing")?;
        let response = response_ref.as_ref().ok_or("response missing")?;
        if payload != response {
            return Err("response did not match payload".into());
        }
        self.await_server()?;
        Ok(())
    }

    /// Verify that a client error was captured.
    ///
    /// # Errors
    /// Returns an error if no failure was observed.
    pub fn verify_error(&self) -> TestResult {
        let error_ref = self.last_error.borrow();
        let err = error_ref
            .as_ref()
            .ok_or("expected client error was not captured")?;
        if !matches!(err, ClientError::Disconnected | ClientError::Io(_)) {
            return Err("unexpected client error variant".into());
        }
        self.await_server()?;
        Ok(())
    }

    fn await_server(&self) -> TestResult {
        if let Some(handle) = self.server.borrow_mut().take() {
            let runtime = self.runtime()?;
            runtime.block_on(async {
                handle
                    .await
                    .map_err(|err| format!("server task failed: {err}"))
            })?;
        }
        Ok(())
    }
}
