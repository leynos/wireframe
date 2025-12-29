//! Unit tests for the wireframe client runtime.
#![expect(
    clippy::excessive_nesting,
    reason = "async closures within builder patterns are inherently nested"
)]

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::{Bytes, BytesMut};
use rstest::rstest;
use socket2::SockRef;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{Decoder, Encoder};

use super::*;
use crate::{
    BincodeSerializer,
    frame::{Endianness, LengthFormat},
};

const MIN_FRAME_LENGTH: usize = 64;
const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_FRAME_LENGTH: usize = 1024;
const KEEPALIVE_DURATION: Duration = Duration::from_secs(30);
const LINGER_DURATION: Duration = Duration::from_secs(1);
const BUFFER_SIZE_U32: u32 = 256 * 1024;
const BUFFER_SIZE_USIZE: usize = 256 * 1024;

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
    A: FnOnce(&WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<TcpStream>>),
{
    let (addr, accept) = spawn_listener().await;

    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await
        .expect("connect client");

    assert_option(&client);

    let _server_stream = accept.await.expect("join accept task");
}

/// Helper function to test lifecycle hooks with a connected client.
async fn test_with_client<F, C>(
    configure_builder: F,
) -> WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<TcpStream>, C>
where
    F: FnOnce(WireframeClientBuilder) -> WireframeClientBuilder<BincodeSerializer, (), C>,
    C: Send + 'static,
{
    let (addr, accept) = spawn_listener().await;
    let client = configure_builder(WireframeClient::builder())
        .connect(addr)
        .await
        .expect("connect client");
    let _server = accept.await.expect("join accept task");
    client
}

use std::{future::Future, pin::Pin};

/// Creates a counter and an incrementing hook for use in lifecycle tests.
///
/// Returns a tuple of the counter and a closure that increments the counter
/// when invoked and returns the provided value.
#[expect(
    clippy::type_complexity,
    reason = "the complex return type is local to tests and extracting a type alias adds indirection"
)]
fn counting_hook<T>() -> (
    Arc<AtomicUsize>,
    impl Fn(T) -> Pin<Box<dyn Future<Output = T> + Send>> + Clone,
)
where
    T: Send + 'static,
{
    let counter = Arc::new(AtomicUsize::new(0));
    let count = counter.clone();

    let increment = move |value: T| {
        let count = count.clone();
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            value
        }) as Pin<Box<dyn Future<Output = T> + Send>>
    };

    (counter, increment)
}

macro_rules! socket_option_test {
    ($name:ident, $configure:expr, $assert:expr $(,)?) => {
        #[tokio::test]
        async fn $name() { assert_builder_option($configure, $assert).await; }
    };
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

socket_option_test!(
    builder_applies_nodelay_option,
    |builder| builder.nodelay(true),
    |client| {
        let stream = client.tcp_stream().nodelay().expect("read nodelay");
        assert!(stream, "expected TCP_NODELAY to be enabled");
    },
);

socket_option_test!(
    builder_applies_keepalive_option,
    |builder| builder.keepalive(Some(KEEPALIVE_DURATION)),
    |client| {
        let sock_ref = SockRef::from(client.tcp_stream());
        assert!(
            sock_ref.keepalive().expect("query SO_KEEPALIVE"),
            "SO_KEEPALIVE should be enabled when configured via builder"
        );
    },
);

socket_option_test!(
    builder_applies_linger_option,
    |builder| builder.linger(Some(LINGER_DURATION)),
    |client| {
        let sock_ref = SockRef::from(client.tcp_stream());
        assert_eq!(
            sock_ref.linger().expect("query SO_LINGER"),
            Some(LINGER_DURATION),
            "SO_LINGER should match builder configuration"
        );
    },
);

socket_option_test!(
    builder_applies_send_buffer_size_option,
    |builder| builder.send_buffer_size(BUFFER_SIZE_U32),
    |client| {
        let sock_ref = SockRef::from(client.tcp_stream());
        assert!(
            sock_ref.send_buffer_size().expect("query SO_SNDBUF") >= BUFFER_SIZE_USIZE,
            "SO_SNDBUF should be at least the requested builder value"
        );
    },
);

socket_option_test!(
    builder_applies_recv_buffer_size_option,
    |builder| builder.recv_buffer_size(BUFFER_SIZE_U32),
    |client| {
        let sock_ref = SockRef::from(client.tcp_stream());
        assert!(
            sock_ref.recv_buffer_size().expect("query SO_RCVBUF") >= BUFFER_SIZE_USIZE,
            "SO_RCVBUF should be at least the requested builder value"
        );
    },
);

socket_option_test!(
    builder_applies_reuseaddr_option,
    |builder| builder.reuseaddr(true),
    |client| {
        let sock_ref = SockRef::from(client.tcp_stream());
        assert!(
            sock_ref.reuse_address().expect("query SO_REUSEADDR"),
            "SO_REUSEADDR should be enabled when configured via builder"
        );
    },
);

#[cfg(all(
    unix,
    not(target_os = "solaris"),
    not(target_os = "illumos"),
    not(target_os = "cygwin"),
))]
socket_option_test!(
    builder_applies_reuseport_option,
    |builder| builder.reuseport(true),
    |client| {
        let sock_ref = SockRef::from(client.tcp_stream());
        assert!(
            sock_ref.reuse_port().expect("query SO_REUSEPORT"),
            "SO_REUSEPORT should be enabled when configured via builder"
        );
    },
);

// ============================================================================
// Lifecycle hook tests
// ============================================================================

#[tokio::test]
async fn setup_callback_invoked_on_connect() {
    let (setup_count, increment) = counting_hook();

    let _client =
        test_with_client(|builder| builder.on_connection_setup(move || increment(42u32))).await;

    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should be invoked exactly once on connect"
    );
}

#[tokio::test]
async fn teardown_callback_receives_setup_state() {
    let teardown_value = Arc::new(AtomicUsize::new(0));
    let value = teardown_value.clone();

    let client = test_with_client(|builder| {
        builder
            .on_connection_setup(|| async { 42usize })
            .on_connection_teardown(move |state| {
                let value = value.clone();
                async move {
                    value.store(state, Ordering::SeqCst);
                }
            })
    })
    .await;

    client.close().await;

    assert_eq!(
        teardown_value.load(Ordering::SeqCst),
        42,
        "teardown callback should receive state from setup"
    );
}

#[tokio::test]
async fn teardown_without_setup_does_not_run() {
    let (teardown_count, increment) = counting_hook();

    let client = test_with_client(|builder| {
        builder.on_connection_teardown(move |value: ()| increment(value))
    })
    .await;

    client.close().await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        0,
        "teardown should not run when no setup hook was configured"
    );
}

#[tokio::test]
async fn error_callback_invoked_on_receive_error() {
    let error_count = Arc::new(AtomicUsize::new(0));
    let count = error_count.clone();

    let (addr, accept) = spawn_listener().await;

    let mut client = WireframeClient::builder()
        .on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    // Drop the server side to cause a disconnection
    let server = accept.await.expect("join accept task");
    drop(server);

    // Try to receive - should fail and invoke error hook
    let result: Result<Vec<u8>, ClientError> = client.receive().await;
    assert!(result.is_err(), "receive should fail after disconnect");

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error callback should be invoked on receive error"
    );
}

#[tokio::test]
async fn setup_and_teardown_callbacks_run() {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let setup = setup_count.clone();
    let teardown = teardown_count.clone();

    let (addr, accept) = spawn_listener().await;

    let client = WireframeClient::builder()
        .on_connection_setup(move || {
            let setup = setup.clone();
            async move {
                setup.fetch_add(1, Ordering::SeqCst);
                "state"
            }
        })
        .on_connection_teardown(move |_: &str| {
            let teardown = teardown.clone();
            async move {
                teardown.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    let _server = accept.await.expect("join accept task");

    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should run exactly once"
    );

    client.close().await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        1,
        "teardown callback should run exactly once"
    );
}
