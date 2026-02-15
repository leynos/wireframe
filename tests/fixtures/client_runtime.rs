//! `ClientRuntimeWorld` fixture for rstest-bdd tests.
//!
//! Provides an echo server/client pair to validate client runtime framing
//! behaviour.

use std::{
    cell::{Cell, RefCell},
    net::SocketAddr,
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use log::warn;
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    BincodeSerializer,
    WireframeError,
    client::{ClientCodecConfig, ClientError, ClientProtocolError, WireframeClient},
    rewind_stream::RewindStream,
};
/// `TestResult` for step definitions.
pub use wireframe_testing::TestResult;

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

    fn block_on<F, T>(&self, future: F) -> TestResult<T>
    where
        F: std::future::Future<Output = T>,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("nested Tokio runtime detected in client runtime fixture".into());
        }
        let runtime = self.runtime()?;
        Ok(runtime.block_on(future))
    }
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq, Clone)]
struct ClientPayload {
    data: Vec<u8>,
}

/// Fixture for `ClientRuntimeWorld`.
// rustfmt collapses simple fixtures into one line, which triggers unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn client_runtime_world() -> ClientRuntimeWorld {
    ClientRuntimeWorld::new()
}

impl ClientRuntimeWorld {
    /// Start an echo server with the specified maximum frame length.
    ///
    /// # Errors
    /// Returns an error if binding or spawning the server fails.
    pub fn start_server(&self, max_frame_length: usize) -> TestResult {
        let listener = self.block_on(async { TcpListener::bind("127.0.0.1:0").await })??;
        let addr = listener.local_addr()?;
        let handle = self.runtime()?.spawn(async move {
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

    /// Start a server that always sends malformed response bytes.
    ///
    /// # Errors
    /// Returns an error if binding or spawning the server fails.
    pub fn start_malformed_response_server(&self) -> TestResult {
        let listener = self.block_on(async { TcpListener::bind("127.0.0.1:0").await })??;
        let addr = listener.local_addr()?;
        let handle = self.runtime()?.spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                warn!("client runtime malformed server failed to accept connection");
                return;
            };
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
            let Some(result) = framed.next().await else {
                warn!("client runtime malformed server closed before receiving a frame");
                return;
            };
            let Ok(_frame) = result else {
                warn!("client runtime malformed server failed to decode request frame");
                return;
            };
            if let Err(err) = framed
                .send(Bytes::from_static(&[0xff, 0xff, 0xff, 0xff]))
                .await
            {
                warn!("client runtime malformed server failed to send invalid frame: {err:?}");
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
        let client = self.block_on(async {
            WireframeClient::builder()
                .codec_config(codec_config)
                .connect(addr)
                .await
        })??;
        *self.client.borrow_mut() = Some(client);
        Ok(())
    }

    /// Send a payload of the specified size and capture the response.
    ///
    /// # Errors
    /// Returns an error if the client is missing or communication fails.
    pub fn send_payload(&self, size: usize) -> TestResult {
        let (payload, result) = self.send_payload_inner(size)?;
        let response = result?;
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
        let (_payload, result) = self.send_payload_inner(size)?;
        match result {
            Ok(_) => return Err("expected client error for oversized payload".into()),
            Err(err) => *self.last_error.borrow_mut() = Some(err),
        }
        Ok(())
    }

    fn send_payload_inner(
        &self,
        size: usize,
    ) -> TestResult<(ClientPayload, Result<ClientPayload, ClientError>)> {
        let payload = ClientPayload {
            data: vec![7_u8; size],
        };
        let mut client = self
            .client
            .borrow_mut()
            .take()
            .ok_or("client not connected")?;
        let result = self.block_on(async { client.call(&payload).await })?;
        *self.client.borrow_mut() = Some(client);
        Ok((payload, result))
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

    /// Verify that the recorded error is a transport `WireframeError`.
    ///
    /// # Errors
    /// Returns an error if no failure was observed or the error variant differs.
    pub fn verify_transport_wireframe_error(&self) -> TestResult {
        let error_ref = self.last_error.borrow();
        let err = error_ref
            .as_ref()
            .ok_or("expected client error was not captured")?;
        if !matches!(err, ClientError::Wireframe(WireframeError::Io(_))) {
            return Err(format!("expected transport WireframeError::Io, got {err:?}").into());
        }
        self.await_server()?;
        Ok(())
    }

    /// Verify that the recorded error is a decode `WireframeError`.
    ///
    /// # Errors
    /// Returns an error if no failure was observed or the error variant differs.
    pub fn verify_decode_wireframe_error(&self) -> TestResult {
        let error_ref = self.last_error.borrow();
        let err = error_ref
            .as_ref()
            .ok_or("expected client error was not captured")?;
        if !matches!(
            err,
            ClientError::Wireframe(WireframeError::Protocol(ClientProtocolError::Deserialize(
                _
            )))
        ) {
            return Err(format!(
                "expected decode WireframeError::Protocol(ClientProtocolError::Deserialize(_)), \
                 got {err:?}"
            )
            .into());
        }
        self.await_server()?;
        Ok(())
    }

    fn await_server(&self) -> TestResult {
        if let Some(handle) = self.server.borrow_mut().take() {
            self.block_on(async {
                handle
                    .await
                    .map_err(|err| format!("server task failed: {err}"))
            })??;
        }
        Ok(())
    }
}
