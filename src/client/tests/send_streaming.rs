//! Unit tests for outbound streaming send API (`send_streaming`).

use std::{io, sync::atomic::Ordering, time::Duration};

use rstest::rstest;

use super::send_streaming_infra::{
    DEFAULT_MAX_FRAME,
    blocking_reader,
    create_send_client,
    create_send_client_with_error_hook,
    create_send_client_with_max_frame,
    protocol_header,
    spawn_dropping_server,
    spawn_receiving_server,
    test_body,
};
use crate::client::{ClientError, SendStreamingConfig, SendStreamingOutcome};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Spawn a server, send a body of `body_size` bytes with the given
/// `chunk_size`, and verify both the client outcome and the number of
/// frames the server received.
async fn test_frames_sent_for_body(
    protocol_header: &[u8],
    body_size: usize,
    chunk_size: usize,
    expected_frames: u64,
) -> TestResult {
    let mut server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let body_vec;
    let body: &[u8] = if body_size == 0 {
        &[]
    } else {
        body_vec = test_body(body_size);
        &body_vec
    };

    let config = SendStreamingConfig::default().with_chunk_size(chunk_size);

    let outcome = client.send_streaming(protocol_header, body, config).await?;

    if outcome.frames_sent() != expected_frames {
        return Err(format!(
            "expected {expected_frames} frames, got {}",
            outcome.frames_sent()
        )
        .into());
    }

    drop(client);
    let received = server.collect_frames().await?;

    let expected_len = usize::try_from(expected_frames)?;
    if received.len() != expected_len {
        return Err(format!(
            "server should receive {expected_frames} frames, got {}",
            received.len()
        )
        .into());
    }

    Ok(())
}

// -----------------------------------------------------------------------
// Core chunking behaviour
// -----------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn emits_correct_number_of_frames(protocol_header: Vec<u8>) -> TestResult {
    test_frames_sent_for_body(&protocol_header, 300, 100, 3).await
}

#[rstest]
#[tokio::test]
async fn frame_payload_contains_correct_body_bytes(protocol_header: Vec<u8>) -> TestResult {
    let mut server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let body = test_body(250);
    let config = SendStreamingConfig::default().with_chunk_size(100);

    let outcome = client
        .send_streaming(&protocol_header, &body[..], config)
        .await?;

    if outcome.frames_sent() != 3 {
        return Err(format!("expected 3 frames, got {}", outcome.frames_sent()).into());
    }

    drop(client);
    let received = server.collect_frames().await?;
    let hlen = protocol_header.len();

    let f0 = received.first().ok_or("missing frame 0")?;
    let f1 = received.get(1).ok_or("missing frame 1")?;
    let f2 = received.get(2).ok_or("missing frame 2")?;

    // First frame: header + body[0..100]
    let f0_body = f0.get(hlen..).ok_or("frame 0 shorter than header")?;
    let expected_0 = body.get(..100).ok_or("body shorter than 100")?;
    if f0_body != expected_0 {
        return Err("frame 0 body mismatch".into());
    }
    // Second frame: header + body[100..200]
    let f1_body = f1.get(hlen..).ok_or("frame 1 shorter than header")?;
    let expected_1 = body.get(100..200).ok_or("body shorter than 200")?;
    if f1_body != expected_1 {
        return Err("frame 1 body mismatch".into());
    }
    // Third frame: header + body[200..250]
    let f2_body = f2.get(hlen..).ok_or("frame 2 shorter than header")?;
    let expected_2 = body.get(200..250).ok_or("body shorter than 250")?;
    if f2_body != expected_2 {
        return Err("frame 2 body mismatch".into());
    }

    Ok(())
}

#[rstest]
#[tokio::test]
async fn exact_chunk_boundary_produces_single_frame(protocol_header: Vec<u8>) -> TestResult {
    test_frames_sent_for_body(&protocol_header, 100, 100, 1).await
}

#[rstest]
#[tokio::test]
async fn partial_final_chunk(protocol_header: Vec<u8>) -> TestResult {
    let mut server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let body = test_body(101);
    let config = SendStreamingConfig::default().with_chunk_size(100);

    let outcome = client
        .send_streaming(&protocol_header, &body[..], config)
        .await?;

    if outcome.frames_sent() != 2 {
        return Err(format!(
            "101 bytes should produce 2 frames, got {}",
            outcome.frames_sent()
        )
        .into());
    }

    drop(client);
    let received = server.collect_frames().await?;
    let hlen = protocol_header.len();
    let last_frame = received.get(1).ok_or("missing frame 1")?;
    let body_len = last_frame
        .len()
        .checked_sub(hlen)
        .ok_or("frame shorter than header")?;
    if body_len != 1 {
        return Err(format!("last frame should have 1 body byte, got {body_len}").into());
    }

    Ok(())
}

#[rstest]
#[tokio::test]
async fn empty_body_sends_zero_frames(protocol_header: Vec<u8>) -> TestResult {
    test_frames_sent_for_body(&protocol_header, 0, 100, 0).await
}

// -----------------------------------------------------------------------
// Chunk size derivation
// -----------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn auto_derives_chunk_size_from_max_frame_length(protocol_header: Vec<u8>) -> TestResult {
    let mut server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let hlen = protocol_header.len();
    let expected_chunk = DEFAULT_MAX_FRAME - hlen;

    // Body is exactly 2 * expected_chunk bytes so we get exactly 2 frames.
    let body = test_body(expected_chunk * 2);
    let config = SendStreamingConfig::default(); // no explicit chunk_size

    let outcome = client
        .send_streaming(&protocol_header, &body[..], config)
        .await?;

    if outcome.frames_sent() != 2 {
        return Err(format!("expected 2 frames, got {}", outcome.frames_sent()).into());
    }

    drop(client);
    let received = server.collect_frames().await?;
    // Each frame should be exactly max_frame_length bytes.
    for (i, frame) in received.iter().enumerate() {
        if frame.len() != DEFAULT_MAX_FRAME {
            return Err(format!(
                "frame {i} should be {DEFAULT_MAX_FRAME} bytes, got {}",
                frame.len()
            )
            .into());
        }
    }

    Ok(())
}

/// Assert that a `send_streaming` result is an I/O error with the
/// expected `ErrorKind`.
fn assert_io_error(
    result: Result<SendStreamingOutcome, ClientError>,
    expected: io::ErrorKind,
) -> TestResult {
    let err = result.err().ok_or("expected error, got Ok")?;
    match &err {
        ClientError::Wireframe(crate::WireframeError::Io(io_err)) => {
            if io_err.kind() != expected {
                return Err(format!("expected {expected:?}, got {:?}", io_err.kind()).into());
            }
        }
        other => {
            return Err(format!("expected Wireframe(Io({expected:?})), got {other:?}").into());
        }
    }
    Ok(())
}

/// Spawn a server, create a client, call `send_streaming` with the given
/// parameters, and assert the result is an I/O error of `expected_error`.
async fn test_send_streaming_rejects(
    header: &[u8],
    body: &[u8],
    config: SendStreamingConfig,
    expected_error: io::ErrorKind,
) -> TestResult {
    let server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;
    assert_io_error(
        client.send_streaming(header, body, config).await,
        expected_error,
    )
}

#[tokio::test]
async fn rejects_oversized_header() -> TestResult {
    let header = vec![0u8; DEFAULT_MAX_FRAME];
    let config = SendStreamingConfig::default();
    test_send_streaming_rejects(&header, b"hello", config, io::ErrorKind::InvalidInput).await
}

#[tokio::test]
async fn rejects_zero_chunk_size() -> TestResult {
    let config = SendStreamingConfig::default().with_chunk_size(0);
    test_send_streaming_rejects(b"\x01", b"hello", config, io::ErrorKind::InvalidInput).await
}

#[tokio::test]
async fn clamps_chunk_size_to_available_capacity() -> TestResult {
    let mut server = spawn_receiving_server().await?;
    // Use a small max frame to make the test deterministic.
    let mut client = create_send_client_with_max_frame(server.addr, 100).await?;

    let header = vec![0xab; 10]; // 10-byte header → 90 bytes available
    let body = test_body(180); // 2 frames at 90 bytes each
    let config = SendStreamingConfig::default().with_chunk_size(9999); // too large

    let outcome = client.send_streaming(&header, &body[..], config).await?;

    if outcome.frames_sent() != 2 {
        return Err(format!("expected 2 frames, got {}", outcome.frames_sent()).into());
    }

    drop(client);
    let received = server.collect_frames().await?;
    for (i, frame) in received.iter().enumerate() {
        if frame.len() > 100 {
            return Err(format!(
                "frame {i} length {} exceeds max_frame_length 100",
                frame.len()
            )
            .into());
        }
    }

    Ok(())
}

// -----------------------------------------------------------------------
// Timeout behaviour
// -----------------------------------------------------------------------

#[tokio::test]
async fn timeout_returns_timed_out() -> TestResult {
    let server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let (reader, _tx) = blocking_reader();

    let config = SendStreamingConfig::default()
        .with_chunk_size(10)
        .with_timeout(Duration::from_millis(50));

    let result = client.send_streaming(b"\x01", reader, config).await;
    assert_io_error(result, io::ErrorKind::TimedOut)
}

// -----------------------------------------------------------------------
// Error hook integration
// -----------------------------------------------------------------------

#[tokio::test]
async fn invokes_error_hook_on_transport_failure() -> TestResult {
    let (server, shutdown_done) = spawn_dropping_server().await?;
    let (mut client, hook_invoked) = create_send_client_with_error_hook(server.addr).await?;

    // Wait for the server to accept and shut down the write side.
    shutdown_done.notified().await;

    // Use a large body to ensure we attempt multiple writes.
    let body = test_body(10_000);
    let config = SendStreamingConfig::default().with_chunk_size(100);

    let result = client.send_streaming(b"\x01", &body[..], config).await;

    if result.is_ok() {
        return Err("expected transport error, got Ok".into());
    }
    if !hook_invoked.load(Ordering::SeqCst) {
        return Err("error hook should be invoked on transport failure".into());
    }

    Ok(())
}

#[tokio::test]
async fn invokes_error_hook_on_timeout() -> TestResult {
    let server = spawn_receiving_server().await?;
    let (mut client, hook_invoked) = create_send_client_with_error_hook(server.addr).await?;

    let (reader, _tx) = blocking_reader();

    let config = SendStreamingConfig::default()
        .with_chunk_size(10)
        .with_timeout(Duration::from_millis(50));

    let result = client.send_streaming(b"\x01", reader, config).await;

    if result.is_ok() {
        return Err("expected timeout error, got Ok".into());
    }
    if !hook_invoked.load(Ordering::SeqCst) {
        return Err("error hook should be invoked on timeout".into());
    }

    Ok(())
}

// -----------------------------------------------------------------------
// Outcome reporting
// -----------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn reports_frames_sent(protocol_header: Vec<u8>) -> TestResult {
    let server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let body = test_body(500);
    let config = SendStreamingConfig::default().with_chunk_size(100);

    let outcome = client
        .send_streaming(&protocol_header, &body[..], config)
        .await?;

    if outcome != SendStreamingOutcome::new(5) {
        return Err(format!("expected 5 frames, got {}", outcome.frames_sent()).into());
    }

    Ok(())
}
