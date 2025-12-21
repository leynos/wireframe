//! Integration tests for the wireframe client runtime.

use futures::StreamExt;
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::client::{ClientCodecConfig, ClientError, WireframeClient};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq)]
struct ClientPayload {
    data: Vec<u8>,
}

#[tokio::test]
async fn client_surfaces_error_when_frame_exceeds_server_max_length() {
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
        let (stream, _) = listener
            .accept()
            .await
            .expect("server accepts connection");
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
        .codec_config(
            ClientCodecConfig::default().max_frame_length(client_max_frame_length),
        )
        .connect(addr)
        .await
        .expect("connect client");
    let payload = ClientPayload {
        data: vec![7_u8; oversized_payload_len],
    };

    let result: Result<ClientPayload, ClientError> = client.call(&payload).await;
    assert!(
        matches!(result, Err(ClientError::Disconnected | ClientError::Io(_))),
        "client should surface transport or disconnect error"
    );

    server.await.expect("join server task");
}
