//! Integration tests for slow reader and writer simulation helpers in
//! `wireframe_testing`.
#![cfg(not(loom))]

use std::{io, num::NonZeroUsize, sync::Arc, time::Duration};

use futures::future::BoxFuture;
use rstest::rstest;
use tokio::task::JoinHandle;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    SlowIoConfig,
    SlowIoPacing,
    decode_frames,
    drive_with_codec_payloads,
    drive_with_slow_codec_payloads,
    drive_with_slow_frames,
    drive_with_slow_payloads,
    encode_frame,
    new_test_codec,
};

const MAX_CAPACITY_PLUS_ONE: usize = (1024 * 1024 * 10) + 1;
type EchoRoute = Arc<dyn Fn(&Envelope) -> BoxFuture<'static, ()> + Send + Sync>;

fn hotline_codec() -> HotlineFrameCodec { HotlineFrameCodec::new(4096) }

fn echo_route() -> EchoRoute {
    Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) })
}

fn panic_route() -> EchoRoute {
    Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> {
        Box::pin(async { panic!("intentional handler panic for test") })
    })
}

fn build_echo_app(
    codec: HotlineFrameCodec,
) -> io::Result<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>> {
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|e| io::Error::other(format!("app init: {e}")))?
        .with_codec(codec)
        .route(1, echo_route())
        .map_err(|e| io::Error::other(format!("route: {e}")))
}

fn build_length_delimited_echo_app() -> io::Result<WireframeApp<BincodeSerializer, (), Envelope>> {
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|e| io::Error::other(format!("app init: {e}")))?
        .route(1, echo_route())
        .map_err(|e| io::Error::other(format!("route: {e}")))
}

fn serialize_envelope(payload: &[u8]) -> io::Result<Vec<u8>> {
    BincodeSerializer
        .serialize(&Envelope::new(1, Some(7), payload.to_vec()))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("serialize: {e}")))
}

fn deserialize_single_envelope(raw: &[u8]) -> io::Result<Envelope> {
    let (env, _) = BincodeSerializer
        .deserialize::<Envelope>(raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("deserialize: {e}")))?;
    Ok(env)
}

fn deserialize_echo_lengths(bytes: &[Vec<u8>]) -> io::Result<Vec<usize>> {
    bytes
        .iter()
        .map(|raw| Ok(deserialize_single_envelope(raw)?.payload_bytes().len()))
        .collect()
}

fn deserialize_echo_payloads(bytes: &[Vec<u8>]) -> io::Result<Vec<Vec<u8>>> {
    bytes
        .iter()
        .map(|raw| Ok(deserialize_single_envelope(raw)?.payload_bytes().to_vec()))
        .collect()
}

fn build_panic_app(
    codec: HotlineFrameCodec,
) -> io::Result<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>> {
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|e| io::Error::other(format!("app init: {e}")))?
        .with_codec(codec)
        .route(1, panic_route())
        .map_err(|e| io::Error::other(format!("route: {e}")))
}

fn join_error(error: &tokio::task::JoinError) -> io::Error {
    io::Error::other(format!("join failed: {error}"))
}

async fn assert_task_pending(task: &JoinHandle<io::Result<Vec<Vec<u8>>>>) -> io::Result<()> {
    tokio::task::yield_now().await;
    if task.is_finished() {
        return Err(io::Error::other("expected paced drive to remain pending"));
    }
    Ok(())
}

async fn run_paced_codec_test(
    payload: Vec<u8>,
    codec: HotlineFrameCodec,
    config: SlowIoConfig,
    final_advance_millis: u64,
) -> io::Result<()> {
    let serialized = serialize_envelope(&payload)?;

    let baseline = drive_with_codec_payloads(
        build_echo_app(codec.clone())?,
        &codec,
        vec![serialized.clone()],
    )
    .await?;
    let baseline_lengths = deserialize_echo_lengths(&baseline)?;
    if baseline_lengths != vec![payload.len()] {
        return Err(io::Error::other(format!(
            "unexpected baseline echo lengths: {baseline_lengths:?}"
        )));
    }

    let paced_app = build_echo_app(codec.clone())?;
    let task = tokio::spawn(async move {
        drive_with_slow_codec_payloads(paced_app, &codec, vec![serialized], config).await
    });

    assert_task_pending(&task).await?;
    tokio::time::advance(Duration::from_millis(20)).await;
    assert_task_pending(&task).await?;

    tokio::time::advance(Duration::from_millis(final_advance_millis)).await;
    let response = task.await.map_err(|error| join_error(&error))??;
    let lengths = deserialize_echo_lengths(&response)?;
    if lengths != vec![payload.len()] {
        return Err(io::Error::other(format!(
            "unexpected paced echo lengths: {lengths:?}"
        )));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn slow_frames_echo_happy_path() -> io::Result<()> {
    let payload_a = serialize_envelope(b"foo")?;
    let payload_b = serialize_envelope(b"bar")?;
    let mut codec = new_test_codec(4096);
    let frame_a = encode_frame(&mut codec, payload_a.clone())?;
    let frame_b = encode_frame(&mut codec, payload_b.clone())?;
    let expected = [frame_a.clone(), frame_b.clone()].concat();
    let config = SlowIoConfig::new()
        .with_writer_pacing(SlowIoPacing::new(
            NonZeroUsize::new(2).ok_or_else(|| io::Error::other("non-zero"))?,
            Duration::ZERO,
        ))
        .with_reader_pacing(SlowIoPacing::new(
            NonZeroUsize::new(3).ok_or_else(|| io::Error::other("non-zero"))?,
            Duration::ZERO,
        ))
        .with_capacity(32);

    let output = drive_with_slow_frames(
        build_length_delimited_echo_app()?,
        vec![frame_a, frame_b],
        config,
    )
    .await?;

    if output != expected {
        return Err(io::Error::other(format!(
            "unexpected raw output bytes: expected {expected:?}, got {output:?}"
        )));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn slow_payloads_echo_happy_path() -> io::Result<()> {
    let expected_payloads = vec![b"hello".to_vec(), b"world".to_vec(), b"slow-io".to_vec()];
    let serialized_payloads = expected_payloads
        .iter()
        .map(|payload| serialize_envelope(payload))
        .collect::<io::Result<Vec<_>>>()?;
    let config = SlowIoConfig::new()
        .with_writer_pacing(SlowIoPacing::new(
            NonZeroUsize::new(3).ok_or_else(|| io::Error::other("non-zero"))?,
            Duration::ZERO,
        ))
        .with_reader_pacing(SlowIoPacing::new(
            NonZeroUsize::new(2).ok_or_else(|| io::Error::other("non-zero"))?,
            Duration::ZERO,
        ))
        .with_capacity(32);

    let output = drive_with_slow_payloads(
        build_length_delimited_echo_app()?,
        serialized_payloads,
        config,
    )
    .await?;
    let frames = decode_frames(output)?;
    let payloads = deserialize_echo_payloads(&frames)?;

    if payloads != expected_payloads {
        return Err(io::Error::other(format!(
            "unexpected echoed payloads: expected {expected_payloads:?}, got {payloads:?}"
        )));
    }
    Ok(())
}

#[rstest]
#[case::slow_writer_delays_inbound_completion((8, vec![b'a'; 64], false, None, 100))]
#[case::slow_reader_delays_outbound_draining((16, vec![b'b'; 256], true, Some(64_usize), 200))]
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn paced_codec_single_payload(
    #[case] case: (usize, Vec<u8>, bool, Option<usize>, u64),
) -> io::Result<()> {
    let (chunk_size, payload, slow_reader, capacity, final_advance_millis) = case;
    let chunk = NonZeroUsize::new(chunk_size).ok_or_else(|| io::Error::other("non-zero"))?;
    let pacing = SlowIoPacing::new(chunk, Duration::from_millis(5));
    let mut config = if slow_reader {
        SlowIoConfig::new().with_reader_pacing(pacing)
    } else {
        SlowIoConfig::new().with_writer_pacing(pacing)
    };
    if let Some(cap) = capacity {
        config = config.with_capacity(cap);
    }
    run_paced_codec_test(payload, hotline_codec(), config, final_advance_millis).await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn combined_slow_reader_and_writer_round_trip_cleanly() -> io::Result<()> {
    let codec = hotline_codec();
    let payload_a = vec![b'c'; 48];
    let payload_b = vec![b'd'; 96];
    let serialized_a = serialize_envelope(&payload_a)?;
    let serialized_b = serialize_envelope(&payload_b)?;
    let app = build_echo_app(codec.clone())?;

    let writer = SlowIoPacing::new(
        NonZeroUsize::new(12).ok_or_else(|| io::Error::other("non-zero"))?,
        Duration::from_millis(5),
    );
    let reader = SlowIoPacing::new(
        NonZeroUsize::new(24).ok_or_else(|| io::Error::other("non-zero"))?,
        Duration::from_millis(5),
    );
    let config = SlowIoConfig::new()
        .with_writer_pacing(writer)
        .with_reader_pacing(reader)
        .with_capacity(64);
    let task = tokio::spawn(async move {
        drive_with_slow_codec_payloads(app, &codec, vec![serialized_a, serialized_b], config).await
    });

    assert_task_pending(&task).await?;
    tokio::time::advance(Duration::from_millis(250)).await;
    let response = task.await.map_err(|error| join_error(&error))??;
    let actual_payloads = deserialize_echo_payloads(&response)?;
    let expected_payloads = vec![payload_a, payload_b];
    if actual_payloads != expected_payloads {
        return Err(io::Error::other(format!(
            "unexpected combined echo payloads: expected {expected_payloads:?}, got \
             {actual_payloads:?}"
        )));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn panic_in_server_is_mapped_to_io_error_other() -> io::Result<()> {
    let codec = hotline_codec();
    let serialized = serialize_envelope(b"panic-test-payload")?;
    let error = drive_with_slow_codec_payloads(
        build_panic_app(codec.clone())?,
        &codec,
        vec![serialized],
        SlowIoConfig::new(),
    )
    .await
    .expect_err("panic should be mapped into io::Error");

    if error.kind() != io::ErrorKind::Other {
        return Err(io::Error::other(format!(
            "expected Other kind for panic mapping, got {:?}",
            error.kind()
        )));
    }

    let message = error.to_string();
    if !message.contains("server task failed") {
        return Err(io::Error::other(format!(
            "panic-mapping error missing preface: {message}"
        )));
    }
    if !message.contains("intentional handler panic for test") {
        return Err(io::Error::other(format!(
            "panic-mapping error missing panic message: {message}"
        )));
    }
    Ok(())
}

#[rstest]
#[case(0, "capacity must be greater than zero")]
#[case(MAX_CAPACITY_PLUS_ONE, "capacity must not exceed 10485760 bytes")]
#[tokio::test(flavor = "current_thread")]
async fn invalid_slow_io_config_is_rejected(
    #[case] capacity: usize,
    #[case] expected: &str,
) -> io::Result<()> {
    let app = build_length_delimited_echo_app()?;
    let error = drive_with_slow_frames(
        app,
        vec![vec![1, 2, 3]],
        SlowIoConfig::new().with_capacity(capacity),
    )
    .await
    .expect_err("invalid config should fail");

    if error.kind() != io::ErrorKind::InvalidInput {
        return Err(io::Error::other(format!(
            "expected InvalidInput, got {:?}",
            error.kind()
        )));
    }
    if error.to_string() != expected {
        return Err(io::Error::other(format!(
            "expected error {expected:?}, got {:?}",
            error.to_string()
        )));
    }
    Ok(())
}
