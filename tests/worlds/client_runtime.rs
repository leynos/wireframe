//! Test world for client runtime scenarios.
#![cfg(not(loom))]

use std::net::SocketAddr;

use cucumber::World;
use futures::{SinkExt, StreamExt};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::client::{ClientCodecConfig, WireframeClient};

use super::TestResult;

#[derive(Debug, Default, World)]
/// Test world exercising the wireframe client runtime.
pub struct ClientRuntimeWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<WireframeClient>,
    payload: Option<ClientPayload>,
    response: Option<ClientPayload>,
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq, Clone)]
struct ClientPayload {
    data: Vec<u8>,
}

impl ClientRuntimeWorld {
    /// Start an echo server with the specified maximum frame length.
    ///
    /// # Errors
    /// Returns an error if binding or spawning the server fails.
    pub async fn start_server(&mut self, max_frame_length: usize) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let codec = LengthDelimitedCodec::builder()
                .max_frame_length(max_frame_length)
                .new_codec();
            let mut framed = Framed::new(stream, codec);
            let Some(result) = framed.next().await else {
                return;
            };
            let Ok(frame) = result else {
                return;
            };
            let _ = framed.send(frame.freeze()).await;
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Connect a client using the specified maximum frame length.
    ///
    /// # Errors
    /// Returns an error if the server has not started or the client fails to connect.
    pub async fn connect_client(&mut self, max_frame_length: usize) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let codec_config = ClientCodecConfig::default().max_frame_length(max_frame_length);
        let client = WireframeClient::builder()
            .codec_config(codec_config)
            .connect(addr)
            .await?;
        self.client = Some(client);
        Ok(())
    }

    /// Send a payload of the specified size and capture the response.
    ///
    /// # Errors
    /// Returns an error if the client is missing or communication fails.
    pub async fn send_payload(&mut self, size: usize) -> TestResult {
        let payload = ClientPayload {
            data: vec![7_u8; size],
        };
        let client = self.client.as_mut().ok_or("client not connected")?;
        let response: ClientPayload = client.call(&payload).await?;
        self.payload = Some(payload);
        self.response = Some(response);
        Ok(())
    }

    /// Verify that the client received the echoed payload.
    ///
    /// # Errors
    /// Returns an error if the response is missing or mismatched.
    pub async fn verify_echo(&mut self) -> TestResult {
        let payload = self.payload.as_ref().ok_or("payload missing")?;
        let response = self.response.as_ref().ok_or("response missing")?;
        if payload != response {
            return Err("response did not match payload".into());
        }
        if let Some(handle) = self.server.take() {
            handle
                .await
                .map_err(|err| format!("server task failed: {err}"))?;
        }
        Ok(())
    }
}
