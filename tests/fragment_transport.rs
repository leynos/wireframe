#![cfg(not(loom))]
//! Integration tests for transport-level fragmentation and reassembly.

use std::{num::NonZeroUsize, time::Duration};

use futures::{SinkExt, StreamExt};
use rstest::rstest;
use thiserror::Error;
use tokio::{
    io::AsyncWriteExt,
    sync::mpsc,
    time::{sleep, timeout},
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, Handler, Packet, PacketParts, WireframeApp},
    fragment::{
        FRAGMENT_MAGIC,
        FragmentationConfig,
        Fragmenter,
        Reassembler,
        ReassemblyError,
        decode_fragment_payload,
        encode_fragment_payload,
    },
    serializer::BincodeSerializer,
};

mod common;
use common::TestResult;

#[derive(Debug, Error)]
enum TestError {
    #[error("test setup failed: {0}")]
    Setup(&'static str),
    #[error("fragmentation failed: {0}")]
    Fragmentation(#[from] wireframe::fragment::FragmentationError),
    #[error("encoding failed: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    #[error("decoding failed: {0}")]
    Decode(#[from] bincode::error::DecodeError),
    #[error("reassembly failed: {0}")]
    Reassembly(#[from] ReassemblyError),
    #[error("send failed: {0}")]
    Send(String),
    #[error("application error: {0}")]
    App(#[from] wireframe::app::WireframeError),
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error("assertion failed: {0}")]
    Assertion(String),
    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("timeout: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("task join failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl<T> From<mpsc::error::SendError<T>> for TestError {
    fn from(err: mpsc::error::SendError<T>) -> Self { TestError::Send(err.to_string()) }
}

const ROUTE_ID: u32 = 42;
const CORRELATION: Option<u64> = Some(7);

fn fragmentation_config(capacity: usize) -> TestResult<FragmentationConfig> {
    let message_limit = NonZeroUsize::new(capacity.saturating_mul(16))
        .ok_or(TestError::Setup("non-zero message limit"))?;

    let config =
        FragmentationConfig::for_frame_budget(capacity, message_limit, Duration::from_millis(30))
            .ok_or(TestError::Setup(
            "frame budget must exceed fragment overhead",
        ))?;

    Ok(config)
}

fn fragmentation_config_with_timeout(
    capacity: usize,
    timeout_ms: u64,
) -> TestResult<FragmentationConfig> {
    let mut config = fragmentation_config(capacity)?;
    config.reassembly_timeout = Duration::from_millis(timeout_ms);
    Ok(config)
}

fn fragment_envelope(env: &Envelope, fragmenter: &Fragmenter) -> TestResult<Vec<Envelope>> {
    let parts = env.clone().into_parts();
    let id = parts.id();
    let correlation = parts.correlation_id();
    let payload = parts.payload();

    if payload.len() <= fragmenter.max_fragment_size().get() {
        return Ok(vec![Envelope::new(id, correlation, payload)]);
    }

    let envelopes = fragmenter
        .fragment_bytes(payload)?
        .into_iter()
        .map(|fragment| {
            let (header, payload) = fragment.into_parts();
            encode_fragment_payload(header, &payload)
                .map(|encoded| Envelope::new(id, correlation, encoded))
                .map_err(TestError::from)
        })
        .collect::<Result<Vec<_>, TestError>>()?;

    Ok(envelopes)
}

async fn send_envelopes(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    envelopes: &[Envelope],
) -> TestResult {
    let serializer = BincodeSerializer;
    for env in envelopes {
        let bytes = serializer.serialize(env)?;
        client.send(bytes.into()).await?;
    }
    Ok(())
}

async fn read_reassembled_response(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    cfg: &FragmentationConfig,
) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);

    while let Some(frame) = client.next().await {
        let bytes = frame?;
        let (env, _) = serializer.deserialize::<Envelope>(&bytes)?;
        let payload = env.into_parts().payload();
        match decode_fragment_payload(&payload)? {
            Some((header, fragment)) => {
                if let Some(message) = reassembler.push(header, fragment)? {
                    return Ok(message.into_payload());
                }
            }
            None => return Ok(payload),
        }
    }

    Err(TestError::Setup("response stream ended before reassembly completed").into())
}

fn make_handler(sender: &mpsc::UnboundedSender<Vec<u8>>) -> Handler<Envelope> {
    let tx = sender.clone();
    std::sync::Arc::new(move |env: &Envelope| {
        let tx = tx.clone();
        let payload = env.clone().into_parts().payload();
        Box::pin(async move {
            assert!(
                tx.send(payload).is_ok(),
                "handler channel send must succeed in tests"
            );
        })
    })
}

fn make_app(
    capacity: usize,
    config: FragmentationConfig,
    sender: &mpsc::UnboundedSender<Vec<u8>>,
) -> TestResult<WireframeApp> {
    Ok(WireframeApp::new()?
        .buffer_capacity(capacity)
        .fragmentation(Some(config))
        .route(ROUTE_ID, make_handler(sender))?)
}

fn spawn_app(
    app: WireframeApp,
) -> (
    Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    tokio::task::JoinHandle<()>,
) {
    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(256);
    let client = Framed::new(client_stream, codec.clone());
    let server = tokio::spawn(async move { app.handle_connection(server_stream).await });
    (client, server)
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn fragmented_request_and_response_round_trip() -> TestResult {
    let buffer_capacity = 512;
    let config = fragmentation_config(buffer_capacity)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx)?;
    let (mut client, server) = spawn_app(app);

    let payload = vec![b'Z'; 1_200];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let fragmenter = Fragmenter::new(config.fragment_payload_cap);
    let fragments = fragment_envelope(&request, &fragmenter)?;

    send_envelopes(&mut client, &fragments).await?;
    client.flush().await?;

    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    assert_eq!(
        observed, payload,
        "observed payload mismatch: expected {payload:?}, got {observed:?}"
    );

    client.get_mut().shutdown().await?;
    let response = read_reassembled_response(&mut client, &config).await?;
    assert_eq!(
        response, payload,
        "response payload mismatch: expected {payload:?}, got {response:?}"
    );

    server.await?;

    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn unfragmented_request_and_response_round_trip() -> TestResult {
    let buffer_capacity = 512;
    let config = fragmentation_config(buffer_capacity)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx)?;
    let (mut client, server) = spawn_app(app);

    let cap = config.fragment_payload_cap.get();
    let payload_len = cap.saturating_sub(8).max(1);
    let payload = vec![b's'; payload_len];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());

    send_envelopes(&mut client, &[request]).await?;
    client.flush().await?;

    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    assert_eq!(
        observed, payload,
        "observed payload mismatch: expected {payload:?}, got {observed:?}"
    );

    client.get_mut().shutdown().await?;
    let response = read_reassembled_response(&mut client, &config).await?;
    assert_eq!(
        response, payload,
        "response payload mismatch: expected {payload:?}, got {response:?}"
    );
    assert!(
        decode_fragment_payload(&response)?.is_none(),
        "small payload should pass through unfragmented"
    );

    server.await?;

    Ok(())
}

struct FragmentRejectionSetup {
    client: Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    server: tokio::task::JoinHandle<()>,
    fragments: Vec<Envelope>,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl FragmentRejectionSetup {
    fn new(
        capacity: usize,
        config: FragmentationConfig,
        fragment_mutator: impl FnOnce(Vec<Envelope>) -> TestResult<Vec<Envelope>>,
    ) -> TestResult<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let app = make_app(capacity, config, &tx)?;
        let (client, server) = spawn_app(app);
        let fragmenter = Fragmenter::new(config.fragment_payload_cap);

        let payload = vec![1_u8; 800];
        let request = Envelope::new(ROUTE_ID, CORRELATION, payload);
        let fragments = fragment_mutator(fragment_envelope(&request, &fragmenter)?)?;

        Ok(Self {
            client,
            server,
            fragments,
            rx,
        })
    }
}

async fn test_fragment_rejection<F>(fragment_mutator: F, rejection_message: &str) -> TestResult
where
    F: FnOnce(Vec<Envelope>) -> TestResult<Vec<Envelope>>,
{
    let buffer_capacity = 512;
    let config = fragmentation_config(buffer_capacity)?;
    let FragmentRejectionSetup {
        mut client,
        server,
        fragments,
        mut rx,
    } = FragmentRejectionSetup::new(buffer_capacity, config, fragment_mutator)?;

    send_envelopes(&mut client, &fragments).await?;
    client.get_mut().shutdown().await?;

    if let Ok(Some(_)) = timeout(Duration::from_millis(200), rx.recv()).await {
        return Err(TestError::Assertion(rejection_message.to_string()).into());
    }

    drop(client);
    server.await?;

    Ok(())
}

type FragmentMutator = fn(Vec<Envelope>) -> TestResult<Vec<Envelope>>;

fn mutate_out_of_order(mut fragments: Vec<Envelope>) -> TestResult<Vec<Envelope>> {
    if fragments.len() < 2 {
        return Err(TestError::Setup("expected at least two fragments").into());
    }

    fragments.swap(0, 1);
    Ok(fragments)
}

fn mutate_duplicate(mut fragments: Vec<Envelope>) -> TestResult<Vec<Envelope>> {
    let duplicate = fragments
        .first()
        .cloned()
        .ok_or(TestError::Setup("fragmenter produced no fragments"))?;
    fragments.insert(1, duplicate);
    Ok(fragments)
}

fn mutate_malformed_header(mut fragments: Vec<Envelope>) -> TestResult<Vec<Envelope>> {
    let parts = fragments
        .first()
        .cloned()
        .ok_or(TestError::Setup(
            "fragmenter must produce at least one fragment",
        ))?
        .into_parts();
    let mut payload = parts.clone().payload();
    if !payload.starts_with(FRAGMENT_MAGIC) {
        return Err(TestError::Assertion("expected fragment to start with marker".into()).into());
    }
    let truncate_len = FRAGMENT_MAGIC.len() + 2;
    if payload.len() > truncate_len {
        payload.truncate(truncate_len);
    } else {
        while payload.len() < truncate_len {
            payload.push(0);
        }
    }
    if let Some(first) = fragments.get_mut(0) {
        *first = Envelope::from_parts(PacketParts::new(
            parts.id(),
            parts.correlation_id(),
            payload,
        ));
    } else {
        return Err(TestError::Setup("fragment list unexpectedly empty").into());
    }
    fragments.truncate(1);
    Ok(fragments)
}

#[rstest]
#[case::out_of_order(
    mutate_out_of_order,
    "handler should not receive out-of-order fragments"
)]
#[case::duplicate(
    mutate_duplicate,
    "handler should not receive after duplicate fragment"
)]
#[case::malformed(mutate_malformed_header, "malformed fragment header is rejected")]
#[tokio::test]
async fn fragment_rejection_cases(
    #[case] mutator: FragmentMutator,
    #[case] rejection_message: &str,
) -> TestResult {
    test_fragment_rejection(mutator, rejection_message).await
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn expired_fragments_are_evicted() -> TestResult {
    let buffer_capacity = 512;
    let timeout_ms = 10;
    let config = fragmentation_config_with_timeout(buffer_capacity, timeout_ms)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx)?;
    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(256);
    let mut client = Framed::new(client_stream, codec.clone());
    let fragmenter = Fragmenter::new(config.fragment_payload_cap);

    let payload = vec![3_u8; 800];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload);
    let fragments = fragment_envelope(&request, &fragmenter)?;

    let server = tokio::spawn(async move { app.handle_connection(server_stream).await });

    // Send the first fragment then pause long enough for eviction.
    let first_fragment = fragments
        .get(..1)
        .ok_or(TestError::Setup("fragmenter produced no fragments"))?;
    send_envelopes(&mut client, first_fragment).await?;
    sleep(Duration::from_millis(timeout_ms * 2)).await;
    if let Some(rest) = fragments.get(1..) {
        send_envelopes(&mut client, rest).await?;
    }
    client.get_mut().shutdown().await?;

    let recv_result = timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(
        recv_result.is_err(),
        "handler should not receive after timeout eviction"
    );

    drop(client);
    server.await?;

    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn fragmentation_can_be_disabled_via_public_api() -> TestResult {
    let capacity = 1024;
    let (tx, mut rx) = mpsc::unbounded_channel();

    let handler = make_handler(&tx);

    let app: WireframeApp = WireframeApp::new()?
        .buffer_capacity(capacity)
        .fragmentation(None)
        .route(ROUTE_ID, handler)?;

    let (mut client, server) = spawn_app(app);

    let half_capacity = capacity
        .checked_div(2)
        .ok_or(TestError::Setup("capacity must be at least two"))?;
    let payload = vec![b'X'; half_capacity];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(&request)?;
    client.send(bytes.into()).await?;
    client.get_mut().shutdown().await?;
    drop(client);

    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    assert_eq!(
        observed, payload,
        "observed payload mismatch: expected {payload:?}, got {observed:?}"
    );

    server.await?;

    Ok(())
}
