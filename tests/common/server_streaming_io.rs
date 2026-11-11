//! In-memory transport helpers shared by the server streaming tests.
//!
//! The back-pressure, idle-producer, and post-buffer scenarios all drive a
//! prepared application over the same duplex transport. They encode requests
//! and decode responses with the same serializer, pace the same channel-one
//! producer, and await the same spawned tasks, so those pieces live here rather
//! than being copied into each test root.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
};
use wireframe::{
    app::Envelope,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestError, TestResult};

/// Spawned producer tasks retained so tests can await them.
pub(crate) type ProducerHandles = Arc<Mutex<Vec<JoinHandle<()>>>>;

/// Encode a request envelope as the frame payload the client would send.
///
/// The correlation identifier is an argument rather than a module constant
/// because each test root owns its own request vocabulary.
pub(crate) fn request_bytes(route: u32, correlation: u64) -> TestResult<Vec<u8>> {
    let envelope = Envelope::new(route, Some(correlation), Vec::new());
    BincodeSerializer
        .serialize(&envelope)
        .map_err(|error| TestError::Msg(format!("serialize request: {error}")))
}

/// Write one length-delimited frame to the duplex peer.
///
/// `put_u32` writes the big-endian length the codec expects, without depending
/// on a platform-sized integer conversion.
pub(crate) async fn write_frame<W>(stream: &mut W, payload: &[u8]) -> TestResult
where
    W: AsyncWrite + Unpin,
{
    let length = u32::try_from(payload.len())
        .map_err(|error| TestError::Msg(format!("frame too long: {error}")))?;
    let mut framed = BytesMut::with_capacity(4 + payload.len());
    framed.put_u32(length);
    framed.extend_from_slice(payload);
    stream.write_all(&framed).await?;
    Ok(())
}

/// Send `frame_count` frames, recording whether each send was accepted.
///
/// An absent `pause` sends as fast as the channel accepts, which is what the
/// back-pressure scenario needs. A pause paces the producer so a test can
/// observe the reader holding it back before the channel accepts another frame.
pub(crate) async fn stream_frames(
    sender: mpsc::Sender<Envelope>,
    results: Arc<Mutex<Vec<bool>>>,
    frame_count: u8,
    pause: Option<Duration>,
) {
    for payload in 1..=frame_count {
        let sent = sender
            .send(Envelope::new(2, None, vec![payload]))
            .await
            .is_ok();
        results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sent);
        if let Some(delay) = pause {
            tokio::time::sleep(delay).await;
        }
    }
}

/// Await every producer task the handler spawned.
pub(crate) async fn join_producers(producers: &ProducerHandles) -> TestResult {
    let handles: Vec<JoinHandle<()>> = producers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .collect();
    for handle in handles {
        handle
            .await
            .map_err(|error| TestError::Msg(format!("producer task failed: {error}")))?;
    }
    Ok(())
}
