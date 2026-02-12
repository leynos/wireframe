//! Integration tests for the wireframe client runtime.
#![cfg(not(loom))]

use std::io;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    WireframeError,
    client::{ClientCodecConfig, ClientError, ClientProtocolError, WireframeClient},
};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq)]
struct ClientPayload {
    data: Vec<u8>,
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq)]
struct Login {
    username: String,
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq)]
struct Ping(u64);

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq)]
struct Metrics {
    values: Vec<u16>,
}

async fn spawn_sample_echo_server() -> io::Result<(std::net::SocketAddr, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let task = tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

        while let Some(Ok(frame)) = framed.next().await {
            if framed.send(frame.freeze()).await.is_err() {
                break;
            }
        }
    });

    Ok((addr, task))
}

#[tokio::test]
async fn client_round_trips_multiple_message_types_through_sample_server() {
    let (addr, server) = spawn_sample_echo_server()
        .await
        .expect("spawn sample echo server");

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let login = Login {
        username: "guest".to_string(),
    };
    let login_resp: Login = client.call(&login).await.expect("login round-trip");
    assert_eq!(login_resp, login);

    let ping = Ping(42);
    let ping_resp: Ping = client.call(&ping).await.expect("ping round-trip");
    assert_eq!(ping_resp, ping);

    let metrics = Metrics {
        values: vec![1, 3, 5, 8],
    };
    let metrics_resp: Metrics = client.call(&metrics).await.expect("metrics round-trip");
    assert_eq!(metrics_resp, metrics);

    server.abort();
}

#[tokio::test]
async fn client_maps_decode_failures_to_wireframe_protocol_errors() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener
        .local_addr()
        .expect("read local address for test listener");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("server accepts connection");
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        let _request = framed.next().await;
        framed
            .send(Bytes::from_static(&[0xff, 0xff, 0xff, 0xff]))
            .await
            .expect("send invalid payload");
    });

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let request = Ping(7);
    let result: Result<Ping, ClientError> = client.call(&request).await;
    assert!(
        matches!(
            result,
            Err(ClientError::Wireframe(WireframeError::Protocol(
                ClientProtocolError::Deserialize(_)
            )))
        ),
        "client should map decode failures to WireframeError::Protocol"
    );

    server.await.expect("join server task");
}

#[tokio::test]
async fn client_maps_transport_disconnect_to_wireframe_io_error() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener
        .local_addr()
        .expect("read local address for test listener");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("server accepts connection");
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        let _request = framed.next().await;
        // Drop without sending a response.
    });

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let payload = Ping(9);
    let result: Result<Ping, ClientError> = client.call(&payload).await;
    assert!(
        matches!(result, Err(ClientError::Wireframe(WireframeError::Io(_)))),
        "client should map transport disconnects to WireframeError::Io"
    );

    server.await.expect("join server task");
}

#[tokio::test]
async fn client_surfaces_oversized_frame_failures_as_wireframe_io() {
    let server_max_frame_length = 64usize;
    let client_max_frame_length = 1024usize;
    let oversized_payload_len = 128usize;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener
        .local_addr()
        .expect("read local address for test listener");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("server accepts connection");
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(server_max_frame_length)
            .new_codec();
        let mut framed = Framed::new(stream, codec);
        let result = framed.next().await;
        assert!(
            matches!(result, Some(Err(_))),
            "server should reject oversized frame"
        );
    });

    let mut client = WireframeClient::builder()
        .codec_config(ClientCodecConfig::default().max_frame_length(client_max_frame_length))
        .connect(addr)
        .await
        .expect("connect client");
    let payload = ClientPayload {
        data: vec![7_u8; oversized_payload_len],
    };

    let result: Result<ClientPayload, ClientError> = client.call(&payload).await;
    assert!(
        matches!(result, Err(ClientError::Wireframe(WireframeError::Io(_)))),
        "client should map oversize transport failures to WireframeError::Io"
    );

    server.await.expect("join server task");
}
