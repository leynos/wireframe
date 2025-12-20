//! Unit tests for the wireframe client runtime.

use rstest::rstest;
use tokio::net::TcpListener;

use super::*;
use crate::frame::Endianness;

const MIN_FRAME_LENGTH: usize = 64;
const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_FRAME_LENGTH: usize = 1024;

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

#[tokio::test]
async fn builder_applies_nodelay_option() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        stream
    });

    let client = WireframeClient::builder()
        .nodelay(true)
        .connect(addr)
        .await
        .expect("connect client");

    let stream = client.framed.get_ref().nodelay().expect("read nodelay");
    assert!(stream, "expected TCP_NODELAY to be enabled");

    let _server_stream = accept.await.expect("join accept task");
}
