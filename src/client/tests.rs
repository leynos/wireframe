//! Unit tests for the wireframe client runtime.

use std::{net::SocketAddr, time::Duration};

use bytes::{Bytes, BytesMut};
use rstest::rstest;
use socket2::SockRef;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{Decoder, Encoder};

use super::*;
use crate::frame::{Endianness, LengthFormat};

const MIN_FRAME_LENGTH: usize = 64;
const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_FRAME_LENGTH: usize = 1024;

async fn spawn_listener() -> (SocketAddr, tokio::task::JoinHandle<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        stream
    });
    (addr, accept)
}

/// Helper function to test that a builder option is correctly applied to the TCP socket.
async fn assert_builder_option<F, A>(configure_builder: F, assert_option: A)
where
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder,
    A: FnOnce(&WireframeClient),
{
    let (addr, accept) = spawn_listener().await;

    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await
        .expect("connect client");

    assert_option(&client);

    let _server_stream = accept.await.expect("join accept task");
}

#[rstest]
#[case(1, MIN_FRAME_LENGTH)]
#[case(MIN_FRAME_LENGTH, MIN_FRAME_LENGTH)]
#[case(MAX_FRAME_LENGTH + 1, MAX_FRAME_LENGTH)]
fn codec_config_clamps_max_frame_length(#[case] input: usize, #[case] expected: usize) {
    let config = ClientCodecConfig::default().max_frame_length(input);
    assert_eq!(config.max_frame_length_value(), expected);
}

#[test]
fn codec_config_defaults_match_server_buffer_capacity() {
    let config = ClientCodecConfig::default();
    assert_eq!(config.max_frame_length_value(), DEFAULT_MAX_FRAME_LENGTH);
    assert_eq!(config.length_format_value().bytes(), 4);
    assert_eq!(config.length_format_value().endianness(), Endianness::Big);
}

#[test]
fn build_codec_configures_length_delimited_codec() {
    let config = ClientCodecConfig::default();
    let mut codec = config.build_codec();

    let payload = Bytes::from_static(b"hello");
    let mut buf = BytesMut::new();

    codec
        .encode(payload.clone(), &mut buf)
        .expect("encoding frame should succeed");

    assert!(
        buf.len() >= 4,
        "encoded frame must at least contain the 4-byte length prefix"
    );

    let bytes = Bytes::from(buf.clone());
    let (len_prefix, data) = bytes.split_at(4);
    let mut expected_prefix = BytesMut::new();
    LengthFormat::u32_be()
        .write_len(payload.len(), &mut expected_prefix)
        .expect("write length prefix");
    let expected_len_prefix = expected_prefix.freeze();
    assert_eq!(
        len_prefix, expected_len_prefix,
        "length prefix should be 4-byte big-endian"
    );
    assert_eq!(
        data, payload,
        "payload bytes after the length prefix should be unchanged"
    );

    let mut decode_buf = buf;
    let decoded = codec
        .decode(&mut decode_buf)
        .expect("decoding frame should succeed")
        .expect("a frame should be produced");

    assert_eq!(decoded, payload, "decoded payload should match original");
}

#[tokio::test]
async fn builder_applies_nodelay_option() {
    assert_builder_option(
        |builder| builder.nodelay(true),
        |client| {
            let stream = client.tcp_stream().nodelay().expect("read nodelay");
            assert!(stream, "expected TCP_NODELAY to be enabled");
        },
    )
    .await;
}

#[tokio::test]
async fn builder_applies_keepalive_option() {
    let keepalive = Duration::from_secs(30);
    assert_builder_option(
        |builder| builder.keepalive(Some(keepalive)),
        |client| {
            let sock_ref = SockRef::from(client.tcp_stream());
            assert!(
                sock_ref.keepalive().expect("query SO_KEEPALIVE"),
                "SO_KEEPALIVE should be enabled when configured via builder"
            );
        },
    )
    .await;
}

#[tokio::test]
async fn builder_applies_linger_option() {
    let linger = Duration::from_secs(1);
    assert_builder_option(
        |builder| builder.linger(Some(linger)),
        |client| {
            let sock_ref = SockRef::from(client.tcp_stream());
            assert_eq!(
                sock_ref.linger().expect("query SO_LINGER"),
                Some(linger),
                "SO_LINGER should match builder configuration"
            );
        },
    )
    .await;
}

#[tokio::test]
async fn builder_applies_send_buffer_size_option() {
    let send_buf_size = 256 * 1024;
    assert_builder_option(
        |builder| builder.send_buffer_size(send_buf_size),
        |client| {
            let sock_ref = SockRef::from(client.tcp_stream());
            assert!(
                sock_ref.send_buffer_size().expect("query SO_SNDBUF") >= send_buf_size as usize,
                "SO_SNDBUF should be at least the requested builder value"
            );
        },
    )
    .await;
}

#[tokio::test]
async fn builder_applies_recv_buffer_size_option() {
    let recv_buf_size = 256 * 1024;
    assert_builder_option(
        |builder| builder.recv_buffer_size(recv_buf_size),
        |client| {
            let sock_ref = SockRef::from(client.tcp_stream());
            assert!(
                sock_ref.recv_buffer_size().expect("query SO_RCVBUF") >= recv_buf_size as usize,
                "SO_RCVBUF should be at least the requested builder value"
            );
        },
    )
    .await;
}

#[tokio::test]
async fn builder_applies_reuseaddr_option() {
    assert_builder_option(
        |builder| builder.reuseaddr(true),
        |client| {
            let sock_ref = SockRef::from(client.tcp_stream());
            assert!(
                sock_ref.reuse_address().expect("query SO_REUSEADDR"),
                "SO_REUSEADDR should be enabled when configured via builder"
            );
        },
    )
    .await;
}

#[cfg(all(
    unix,
    not(target_os = "solaris"),
    not(target_os = "illumos"),
    not(target_os = "cygwin"),
))]
#[tokio::test]
async fn builder_applies_reuseport_option() {
    assert_builder_option(
        |builder| builder.reuseport(true),
        |client| {
            let sock_ref = SockRef::from(client.tcp_stream());
            assert!(
                sock_ref.reuse_port().expect("query SO_REUSEPORT"),
                "SO_REUSEPORT should be enabled when configured via builder"
            );
        },
    )
    .await;
}
