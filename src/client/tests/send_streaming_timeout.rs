//! Deadline enforcement tests for the outbound streaming send API.
//!
//! These tests pin the observable effects of `SendStreamingConfig::timeout`:
//! expiring before any read must emit nothing, expiring after a chunk must emit
//! exactly that chunk, and both paths must run the error hook. They live apart
//! from the chunking tests so each module stays within the size limit.

use std::{io, sync::atomic::Ordering, time::Duration};

use bytes::Bytes;
use tokio::time::timeout;

use super::{
    send_streaming::assert_io_error,
    send_streaming_infra::{
        blocking_reader,
        create_send_client,
        create_send_client_with_error_hook,
        spawn_receiving_server,
        test_body,
    },
};
use crate::client::SendStreamingConfig;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Outer bound for a timeout scenario. It is far longer than any configured
/// deadline here, so a `send_streaming` that ignores its deadline fails this
/// test promptly instead of hanging the suite.
const TIMEOUT_WATCHDOG: Duration = Duration::from_secs(2);

/// An expired deadline before the first read must fail the call without
/// emitting any frame, not even a header-only prefix.
///
/// The sender stays alive so dropping it cannot turn the pending read into an
/// end-of-stream that completes the loop before the deadline fires.
#[tokio::test]
async fn timeout_returns_timed_out() -> TestResult {
    let mut server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let (reader, _tx) = blocking_reader();

    let config = SendStreamingConfig::default()
        .with_chunk_size(10)
        .with_timeout(Duration::from_millis(50));

    let result = timeout(
        TIMEOUT_WATCHDOG,
        client.send_streaming(b"\x01", reader, config),
    )
    .await
    .map_err(|_| "send_streaming ignored its configured timeout")?;
    assert_io_error(result, io::ErrorKind::TimedOut)?;

    drop(client);
    let received = server.collect_frames().await?;
    if !received.is_empty() {
        return Err(format!(
            "a timeout before the first read must emit no frames, got {received:?}"
        )
        .into());
    }
    Ok(())
}

/// A chunk already available at call time must be emitted exactly once, and
/// the deadline must stop the loop before the next pending read adds a frame.
///
/// The deadline is real rather than paused: with `start_paused` the timer can
/// advance while the socket write is still queued, which would make the frame
/// count depend on scheduling instead of on the code under test.
#[tokio::test]
async fn timeout_after_partial_progress_stops_further_frames() -> TestResult {
    let mut server = spawn_receiving_server().await?;
    let mut client = create_send_client(server.addr).await?;

    let chunk_size = 10;
    let first_chunk = Bytes::from(test_body(chunk_size));
    let (reader, sender) = blocking_reader();

    // One complete chunk is queued, so the first read succeeds and emits its
    // frame. Keeping the sender alive leaves every later read pending.
    sender
        .send(Ok(first_chunk.clone()))
        .await
        .map_err(|_| "the pre-queued chunk should be accepted")?;

    let config = SendStreamingConfig::default()
        .with_chunk_size(chunk_size)
        .with_timeout(Duration::from_millis(200));

    let result = timeout(
        TIMEOUT_WATCHDOG,
        client.send_streaming(b"\x01", reader, config),
    )
    .await
    .map_err(|_| "send_streaming ignored its configured timeout")?;
    assert_io_error(result, io::ErrorKind::TimedOut)?;

    drop(sender);
    drop(client);
    let received = server.collect_frames().await?;

    // Collected frames prove the decoded count only. Cancellation can truncate
    // a frame that was partly written, so this does not prove the absence of
    // trailing wire bytes.
    if received.len() != 1 {
        return Err(format!(
            "the timeout must stop emission after the one complete frame, got {} frames",
            received.len()
        )
        .into());
    }
    let expected_frame = [b"\x01".as_slice(), first_chunk.as_ref()].concat();
    if received.first() != Some(&expected_frame) {
        return Err(format!(
            "the emitted frame must carry the protocol header and the first chunk, got {:?}",
            received.first()
        )
        .into());
    }
    Ok(())
}

/// A timeout must still invoke the error hook, matching a transport failure.
#[tokio::test]
async fn invokes_error_hook_on_timeout() -> TestResult {
    let server = spawn_receiving_server().await?;
    let (mut client, hook_invoked) = create_send_client_with_error_hook(server.addr).await?;

    let (reader, _tx) = blocking_reader();

    let config = SendStreamingConfig::default()
        .with_chunk_size(10)
        .with_timeout(Duration::from_millis(50));

    // Without this bound a `send_streaming` that ignores its deadline blocks
    // forever, turning a mutation failure into an apparent test hang.
    let result = timeout(
        TIMEOUT_WATCHDOG,
        client.send_streaming(b"\x01", reader, config),
    )
    .await
    .map_err(|_| "send_streaming ignored its configured timeout")?;

    if result.is_ok() {
        return Err("expected timeout error, got Ok".into());
    }
    if !hook_invoked.load(Ordering::SeqCst) {
        return Err("error hook should be invoked on timeout".into());
    }

    Ok(())
}
