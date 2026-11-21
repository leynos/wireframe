//! Integration tests for the wireframe client runtime.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::io;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::rstest;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    WireframeError,
    app::Envelope,
    client::{ClientCodecConfig, ClientError, ClientProtocolError, WireframeClient},
    correlation::CorrelatableFrame,
    message::Message,
    test_helpers::ServerTaskGuard,
};

#[path = "../examples/support/echo_login_contract.rs"]
mod echo_login_contract;

use echo_login_contract::{LOGIN_ROUTE_ID, LoginAck, LoginRequest};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq)]
struct ClientPayload {
    data: Vec<u8>,
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

/// Error type used by the guarded bodies in these tests.
type BodyError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::test]
async fn client_round_trips_multiple_message_types_through_sample_server() {
    let (addr, server) = spawn_sample_echo_server()
        .await
        .expect("spawn sample echo server");

    // The sample server serves until its client disconnects, so it is aborted
    // and joined rather than awaited. Connection setup and every assertion run
    // inside the guarded body: a failed round-trip still reaps the server, and
    // its panic is resumed afterwards.
    let ((), server_outcome) = ServerTaskGuard::new(server)
        .run_and_abort(async move {
            let mut client = WireframeClient::builder().connect(addr).await?;

            let login = LoginRequest {
                username: "guest".to_owned(),
            };
            let login_resp: LoginRequest = client.call(&login).await?;
            assert_eq!(login_resp, login, "login_resp should equal login");

            let ping = Ping(42);
            let ping_resp: Ping = client.call(&ping).await?;
            assert_eq!(ping_resp, ping, "ping_resp should equal ping");

            let metrics = Metrics {
                values: vec![1, 3, 5, 8],
            };
            let metrics_resp: Metrics = client.call(&metrics).await?;
            assert_eq!(metrics_resp, metrics, "metrics_resp should equal metrics");

            drop(client);
            Ok::<(), BodyError>(())
        })
        .await
        .expect("the guarded sample server body must not fail");

    assert!(
        server_outcome.is_ok(),
        "the sample server must not panic: {server_outcome:?}"
    );
}

#[rstest]
#[case("guest")]
#[case("operator")]
#[tokio::test]
async fn client_decodes_echoed_login_acknowledgement(#[case] username: &str) {
    let (addr, server) = spawn_sample_echo_server()
        .await
        .expect("spawn sample echo server");

    let ((), server_outcome) = ServerTaskGuard::new(server)
        .run_and_abort(async move {
            let mut client = WireframeClient::builder().connect(addr).await?;

            let login = LoginRequest {
                username: username.to_owned(),
            };
            let request = Envelope::new(LOGIN_ROUTE_ID, None, login.to_bytes()?);
            let response: Envelope = client.call_correlated(request).await?;
            let (ack, _) = LoginAck::from_bytes(response.payload_bytes())?;
            assert_eq!(ack.username, username, "ack.username should equal username");
            assert!(
                response.correlation_id().is_some(),
                "assertion failed: response.correlation_id().is_some()"
            );

            drop(client);
            Ok::<(), BodyError>(())
        })
        .await
        .expect("the guarded login acknowledgement body must not fail");

    assert!(
        server_outcome.is_ok(),
        "the sample server must not panic: {server_outcome:?}"
    );
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
