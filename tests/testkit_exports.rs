//! Integration tests for the `wireframe::testkit` root export.
#![cfg(not(loom))]

use std::{io, num::NonZeroUsize, sync::Arc, time::Duration};

use futures::future::BoxFuture;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
    fragment::Fragmenter,
    message_assembler::{
        AssembledMessage,
        EnvelopeId,
        EnvelopeRouting,
        MessageAssemblyError,
        MessageKey,
    },
    serializer::{BincodeSerializer, Serializer},
    testkit::{
        FragmentReassemblyErrorExpectation,
        FragmentReassemblySnapshot,
        MAX_SLOW_IO_CAPACITY,
        MessageAssemblySnapshot,
        SlowIoConfig,
        SlowIoPacing,
        assert_fragment_reassembly_completed_bytes,
        assert_fragment_reassembly_error,
        assert_message_assembly_completed_for_key,
        drive_with_fragments,
        drive_with_partial_frames,
        drive_with_slow_codec_payloads,
    },
};

fn hotline_codec() -> HotlineFrameCodec { HotlineFrameCodec::new(4096) }

fn build_echo_app(
    codec: HotlineFrameCodec,
) -> io::Result<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>> {
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|error| io::Error::other(format!("app init: {error}")))?
        .with_codec(codec)
        .route(
            1,
            Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) }),
        )
        .map_err(|error| io::Error::other(format!("route: {error}")))
}

fn serialize_envelope(payload: &[u8]) -> io::Result<Vec<u8>> {
    BincodeSerializer
        .serialize(&Envelope::new(1, Some(7), payload.to_vec()))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("serialize: {error}")))
}

fn deserialize_payload(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let (envelope, consumed) =
        BincodeSerializer
            .deserialize::<Envelope>(bytes)
            .map_err(|error| {
                io::Error::new(io::ErrorKind::InvalidData, format!("deserialize: {error}"))
            })?;
    if consumed != bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "deserialize: trailing bytes after envelope: consumed {consumed} of {}",
                bytes.len()
            ),
        ));
    }
    Ok(envelope.payload_bytes().to_vec())
}

#[tokio::test]
async fn root_testkit_exports_partial_frame_and_fragment_drivers() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let payload = serialize_envelope(b"partial-frame")?;
    let chunk = NonZeroUsize::new(3).ok_or_else(|| io::Error::other("chunk must be non-zero"))?;

    let response_payloads = drive_with_partial_frames(app, &codec, vec![payload], chunk).await?;
    let first_payload = response_payloads
        .first()
        .ok_or_else(|| io::Error::other("expected response payload"))?;
    let echoed_payload = deserialize_payload(first_payload)?;
    if echoed_payload != b"partial-frame" {
        return Err(io::Error::other(format!(
            "unexpected echoed payload: {echoed_payload:?}"
        )));
    }

    let app = build_echo_app(codec.clone())?;
    let fragmenter = Fragmenter::new(
        NonZeroUsize::new(16).ok_or_else(|| io::Error::other("fragment cap must be non-zero"))?,
    );
    let payload = serialize_envelope(b"fragment-test")?;
    let response_payloads = drive_with_fragments(app, &codec, &fragmenter, payload).await?;
    let first_payload = response_payloads
        .first()
        .ok_or_else(|| io::Error::other("expected response payload"))?;
    let echoed_payload = deserialize_payload(first_payload)?;
    if echoed_payload != b"fragment-test" {
        return Err(io::Error::other(format!(
            "unexpected fragmented echoed payload: {echoed_payload:?}"
        )));
    }
    Ok(())
}

async fn advance_until_finished<T: Send + 'static>(
    task: &tokio::task::JoinHandle<T>,
    step: Duration,
    timeout: Duration,
) -> io::Result<()> {
    let mut total = Duration::ZERO;
    while !task.is_finished() && total < timeout {
        tokio::time::advance(step).await;
        total += step;
    }
    if !task.is_finished() {
        return Err(io::Error::other(format!(
            "task did not complete after {total:?}"
        )));
    }
    Ok(())
}

fn assert_echoed_payload(payloads: &[Vec<u8>], expected: &[u8]) -> io::Result<()> {
    let first = payloads
        .first()
        .ok_or_else(|| io::Error::other("expected response payload"))?;
    let echoed = deserialize_payload(first)?;
    if echoed != expected {
        return Err(io::Error::other(format!(
            "unexpected paced payload: {echoed:?}"
        )));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn root_testkit_exports_slow_io_helpers() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let payload = serialize_envelope(b"slow-io")?;
    let config = SlowIoConfig::new()
        .with_writer_pacing(SlowIoPacing::new(
            NonZeroUsize::new(2).expect("2 is non-zero"),
            Duration::from_millis(5),
        ))
        .with_reader_pacing(SlowIoPacing::new(
            NonZeroUsize::new(4).expect("4 is non-zero"),
            Duration::from_millis(5),
        ))
        .with_capacity(64);

    let task = tokio::spawn(async move {
        drive_with_slow_codec_payloads(app, &codec, vec![payload], config).await
    });

    tokio::task::yield_now().await;
    if task.is_finished() {
        return Err(io::Error::other("expected paced helper to remain pending"));
    }

    advance_until_finished(&task, Duration::from_millis(10), Duration::from_secs(1)).await?;

    let response_payloads = task
        .await
        .map_err(|error| io::Error::other(format!("join failed: {error}")))??;
    assert_echoed_payload(&response_payloads, b"slow-io")
}

#[test]
fn root_testkit_exports_reassembly_assertions() -> wireframe::testkit::TestResult {
    let routing = EnvelopeRouting {
        envelope_id: EnvelopeId(1),
        correlation_id: None,
    };
    let assembled = AssembledMessage::new(MessageKey(7), routing, vec![], b"done".to_vec());
    let last_result: Result<Option<AssembledMessage>, MessageAssemblyError> =
        Ok(Some(assembled.clone()));
    let completed = [assembled];
    let snapshot = MessageAssemblySnapshot::new(Some(&last_result), &completed, &[], 0, 0);
    assert_message_assembly_completed_for_key(snapshot, MessageKey(7), b"done")?;

    let reassembled = wireframe::fragment::ReassembledMessage::new(
        wireframe::fragment::MessageId::new(9),
        b"fragment".to_vec(),
    );
    let fragment_snapshot = FragmentReassemblySnapshot::new(Some(&reassembled), None, &[], 0);
    assert_fragment_reassembly_completed_bytes(fragment_snapshot, b"fragment")?;

    let fragment_error = wireframe::fragment::ReassemblyError::MessageTooLarge {
        message_id: wireframe::fragment::MessageId::new(11),
        attempted: 99,
        limit: std::num::NonZeroUsize::MIN,
    };
    let error_snapshot = FragmentReassemblySnapshot::new(None, Some(&fragment_error), &[], 0);
    assert_fragment_reassembly_error(
        error_snapshot,
        FragmentReassemblyErrorExpectation::MessageTooLarge {
            message_id: wireframe::fragment::MessageId::new(11),
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// SlowIoConfig validation failure-path tests
// ---------------------------------------------------------------------------

fn assert_slow_io_config_rejects(config: SlowIoConfig, context: &str) -> io::Result<()> {
    match config.validate() {
        Ok(_) => Err(io::Error::other(context)),
        Err(err) if err.kind() == io::ErrorKind::InvalidInput => Ok(()),
        Err(err) => Err(io::Error::other(format!(
            "{context}: expected InvalidInput, got {:?}",
            err.kind()
        ))),
    }
}

#[test]
fn slow_io_config_validate_rejects_zero_capacity() -> io::Result<()> {
    assert_slow_io_config_rejects(
        SlowIoConfig::new().with_capacity(0),
        "expected validate to fail for zero capacity",
    )
}

#[test]
fn slow_io_config_validate_rejects_capacity_over_max() -> io::Result<()> {
    assert_slow_io_config_rejects(
        SlowIoConfig::new().with_capacity(MAX_SLOW_IO_CAPACITY + 1),
        "expected validate to fail for capacity over max",
    )
}

fn assert_slow_io_rejects_chunk_exceeding_capacity<F>(attach_pacing: F) -> io::Result<()>
where
    F: FnOnce(SlowIoConfig, SlowIoPacing) -> SlowIoConfig,
{
    let capacity = 4;
    let chunk_size =
        NonZeroUsize::new(capacity * 2).ok_or_else(|| io::Error::other("8 is non-zero"))?;
    let pacing = SlowIoPacing::new(chunk_size, Duration::from_millis(1));
    let config = attach_pacing(SlowIoConfig::new().with_capacity(capacity), pacing);
    assert_slow_io_config_rejects(
        config,
        "expected validate to fail when chunk exceeds capacity",
    )
}

#[test]
fn slow_io_config_validate_rejects_writer_chunk_exceeding_capacity() -> io::Result<()> {
    assert_slow_io_rejects_chunk_exceeding_capacity(|config, pacing| {
        config.with_writer_pacing(pacing)
    })
}

#[test]
fn slow_io_config_validate_rejects_reader_chunk_exceeding_capacity() -> io::Result<()> {
    assert_slow_io_rejects_chunk_exceeding_capacity(|config, pacing| {
        config.with_reader_pacing(pacing)
    })
}
