#![cfg(not(loom))]

use std::{num::NonZeroUsize, time::Duration};

use futures::{SinkExt, StreamExt};
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
        decode_fragment_payload,
        encode_fragment_payload,
    },
    serializer::BincodeSerializer,
};

const ROUTE_ID: u32 = 42;
const CORRELATION: Option<u64> = Some(7);

fn fragmentation_config(capacity: usize) -> FragmentationConfig {
    FragmentationConfig::for_frame_budget(
        capacity,
        NonZeroUsize::new(capacity * 16).expect("non-zero message limit"),
        Duration::from_millis(30),
    )
    .expect("frame budget must exceed fragment overhead")
}

fn fragment_envelope(env: &Envelope, fragmenter: &Fragmenter) -> Vec<Envelope> {
    let parts = env.clone().into_parts();
    let id = parts.id();
    let correlation = parts.correlation_id();
    let payload = parts.payload();

    if payload.len() <= fragmenter.max_fragment_size().get() {
        return vec![Envelope::new(id, correlation, payload)];
    }

    fragmenter
        .fragment_bytes(payload)
        .expect("fragment payload")
        .into_iter()
        .map(|fragment| {
            let (header, payload) = fragment.into_parts();
            let encoded = encode_fragment_payload(header, &payload).expect("encode fragment");
            Envelope::new(id, correlation, encoded)
        })
        .collect()
}

async fn send_envelopes(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    envelopes: &[Envelope],
) {
    let serializer = BincodeSerializer;
    for env in envelopes {
        let bytes = serializer.serialize(env).expect("serialize envelope");
        client.send(bytes.into()).await.expect("send frame");
    }
}

async fn read_reassembled_response(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    cfg: &FragmentationConfig,
) -> Vec<u8> {
    let serializer = BincodeSerializer;
    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);

    while let Some(frame) = client.next().await {
        let bytes = frame.expect("read frame");
        let (env, _) = serializer
            .deserialize::<Envelope>(&bytes)
            .expect("decode envelope");
        let payload = env.into_parts().payload();
        if let Some((header, fragment)) =
            decode_fragment_payload(&payload).expect("decode fragment payload")
        {
            if let Some(message) = reassembler
                .push(header, fragment)
                .expect("reassemble fragment")
            {
                return message.into_payload();
            }
        } else {
            return payload;
        }
    }

    panic!("response stream ended before reassembly completed");
}

fn make_handler(sender: &mpsc::UnboundedSender<Vec<u8>>) -> Handler<Envelope> {
    let tx = sender.clone();
    std::sync::Arc::new(move |env: &Envelope| {
        let tx = tx.clone();
        let payload = env.clone().into_parts().payload();
        Box::pin(async move {
            tx.send(payload).expect("record payload");
        })
    })
}

fn make_app(
    capacity: usize,
    config: FragmentationConfig,
    sender: &mpsc::UnboundedSender<Vec<u8>>,
) -> WireframeApp {
    WireframeApp::new()
        .expect("build app")
        .buffer_capacity(capacity)
        .fragmentation(Some(config))
        .route(ROUTE_ID, make_handler(sender))
        .expect("register route")
}

#[tokio::test]
async fn fragmented_request_and_response_round_trip() {
    let buffer_capacity = 512;
    let config = fragmentation_config(buffer_capacity);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx);

    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(256);
    let mut client = Framed::new(client_stream, codec.clone());

    let server = tokio::spawn(async move { app.handle_connection(server_stream).await });

    let payload = vec![b'Z'; 1_200];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let fragmenter = Fragmenter::new(config.fragment_payload_cap);
    let fragments = fragment_envelope(&request, &fragmenter);

    send_envelopes(&mut client, &fragments).await;
    client.flush().await.expect("flush client");

    let observed = rx.recv().await.expect("handler payload");
    assert_eq!(observed, payload);

    client.get_mut().shutdown().await.expect("shutdown write");
    let response = read_reassembled_response(&mut client, &config).await;
    assert_eq!(response, payload);

    server.await.expect("server task");
}

#[tokio::test]
async fn unfragmented_request_and_response_round_trip() {
    let buffer_capacity = 512;
    let config = fragmentation_config(buffer_capacity);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx);

    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(256);
    let mut client = Framed::new(client_stream, codec.clone());

    let server = tokio::spawn(async move { app.handle_connection(server_stream).await });

    let cap = config.fragment_payload_cap.get();
    let payload_len = cap.saturating_sub(8).max(1);
    let payload = vec![b's'; payload_len];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());

    send_envelopes(&mut client, &[request]).await;
    client.flush().await.expect("flush client");

    let observed = rx.recv().await.expect("handler payload");
    assert_eq!(observed, payload);

    client.get_mut().shutdown().await.expect("shutdown write");
    let response = read_reassembled_response(&mut client, &config).await;
    assert_eq!(response, payload);
    assert!(
        matches!(decode_fragment_payload(&response), Ok(None)),
        "small payload should pass through unfragmented"
    );

    server.await.expect("server task");
}

struct FragmentRejectionSetup {
    client: Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    server: tokio::task::JoinHandle<()>,
    fragments: Vec<Envelope>,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl FragmentRejectionSetup {
    fn new(config: FragmentationConfig, fragment_mutator: impl FnOnce(&mut Vec<Envelope>)) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let app = make_app(config.fragment_payload_cap.get(), config, &tx);
        let codec = app.length_codec();
        let (client_stream, server_stream) = tokio::io::duplex(256);
        let client = Framed::new(client_stream, codec.clone());
        let server = tokio::spawn(async move { app.handle_connection(server_stream).await });
        let fragmenter = Fragmenter::new(config.fragment_payload_cap);

        let payload = vec![1_u8; 800];
        let request = Envelope::new(ROUTE_ID, CORRELATION, payload);
        let mut fragments = fragment_envelope(&request, &fragmenter);
        fragment_mutator(&mut fragments);

        Self {
            client,
            server,
            fragments,
            rx,
        }
    }
}

async fn test_fragment_rejection<F>(fragment_mutator: F, rejection_message: &str)
where
    F: FnOnce(&mut Vec<Envelope>),
{
    let config = fragmentation_config(512);
    let FragmentRejectionSetup {
        mut client,
        server,
        fragments,
        mut rx,
    } = FragmentRejectionSetup::new(config, fragment_mutator);

    send_envelopes(&mut client, &fragments).await;
    client.get_mut().shutdown().await.expect("shutdown write");

    assert!(
        timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "{rejection_message}"
    );

    drop(client);
    server.await.expect("server task");
}

#[tokio::test]
async fn out_of_order_fragments_are_rejected() {
    test_fragment_rejection(
        |fragments| fragments.swap(0, 1),
        "handler should not receive out-of-order fragments",
    )
    .await;
}

#[tokio::test]
async fn duplicate_fragments_clear_reassembly() {
    test_fragment_rejection(
        |fragments| {
            let duplicate = fragments[0].clone();
            fragments.insert(1, duplicate);
        },
        "handler should not receive after duplicate fragment",
    )
    .await;
}

#[tokio::test]
async fn fragment_rejection_malformed_fragment_header() {
    test_fragment_rejection(
        |fragments| {
            let parts = fragments
                .first()
                .cloned()
                .expect("fragmenter must produce at least one fragment")
                .into_parts();
            let mut payload = parts.clone().payload();
            assert!(
                payload.starts_with(FRAGMENT_MAGIC),
                "expected fragment to start with marker"
            );
            let truncate_len = FRAGMENT_MAGIC.len() + 2;
            if payload.len() > truncate_len {
                payload.truncate(truncate_len);
            } else {
                while payload.len() < truncate_len {
                    payload.push(0);
                }
            }
            fragments[0] = Envelope::from_parts(PacketParts::new(
                parts.id(),
                parts.correlation_id(),
                payload,
            ));
            fragments.truncate(1);
        },
        "malformed fragment header is rejected",
    )
    .await;
}

#[tokio::test]
async fn expired_fragments_are_evicted() {
    let buffer_capacity = 512;
    let timeout_ms = 10;
    let config = FragmentationConfig::for_frame_budget(
        buffer_capacity,
        NonZeroUsize::new(buffer_capacity * 2).expect("non-zero message limit"),
        Duration::from_millis(timeout_ms),
    )
    .expect("frame budget must exceed fragment overhead");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx);
    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(256);
    let mut client = Framed::new(client_stream, codec.clone());
    let fragmenter = Fragmenter::new(config.fragment_payload_cap);

    let payload = vec![3_u8; 800];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload);
    let fragments = fragment_envelope(&request, &fragmenter);

    let server = tokio::spawn(async move { app.handle_connection(server_stream).await });

    // Send the first fragment then pause long enough for eviction.
    send_envelopes(&mut client, &fragments[..1]).await;
    sleep(Duration::from_millis(timeout_ms * 2)).await;
    send_envelopes(&mut client, &fragments[1..]).await;
    client.get_mut().shutdown().await.expect("shutdown write");

    assert!(
        timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "handler should not receive after timeout eviction"
    );

    drop(client);
    server.await.expect("server task");
}

#[tokio::test]
async fn fragmentation_can_be_disabled_via_public_api() {
    let capacity = 1024;
    let (tx, mut rx) = mpsc::unbounded_channel();

    let handler = make_handler(&tx);

    let app: WireframeApp = WireframeApp::new()
        .expect("build app")
        .buffer_capacity(capacity)
        .fragmentation(None)
        .route(ROUTE_ID, handler)
        .expect("register route");

    let codec = app.length_codec();
    let (client_stream, server_stream) = tokio::io::duplex(256);
    let mut client = Framed::new(client_stream, codec.clone());
    let server = tokio::spawn(async move { app.handle_connection(server_stream).await });

    let payload = vec![b'X'; capacity / 2];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(&request).expect("serialize envelope");
    client.send(bytes.into()).await.expect("send frame");
    client.get_mut().shutdown().await.expect("shutdown write");
    drop(client);

    let observed = rx.recv().await.expect("handler payload");
    assert_eq!(observed, payload);

    server.await.expect("server task");
}
