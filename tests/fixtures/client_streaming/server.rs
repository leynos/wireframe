//! Test server helper functions for client streaming BDD tests.

use futures::SinkExt;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::types::{CorrelationId, MessageId, Payload, StreamTestEnvelope};

/// Send a single frame with a mismatched correlation ID.
pub(crate) async fn send_mismatch_frame<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
    cid: CorrelationId,
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let bad = StreamTestEnvelope::data(
        MessageId::new(1),
        CorrelationId::new(cid.get() + 999),
        Payload::new(vec![99]),
    );
    if let Ok(encoded) = bad.serialize_to_bytes() {
        let _ = framed.send(encoded).await;
    }
}

/// Send `count` data frames with payload `[1], [2], ..., [count]`.
pub(crate) async fn send_data_frames<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
    cid: CorrelationId,
    count: usize,
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    for i in 1..=count {
        let Ok(id) = u32::try_from(i) else { break };
        let Ok(payload_byte) = u8::try_from(i) else {
            break;
        };
        let frame =
            StreamTestEnvelope::data(MessageId::new(id), cid, Payload::new(vec![payload_byte]));
        let Ok(encoded) = frame.serialize_to_bytes() else {
            break;
        };
        if framed.send(encoded).await.is_err() {
            break;
        }
    }
}

/// Send `count` data frames followed by a terminator.
pub(crate) async fn send_data_and_terminator<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
    cid: CorrelationId,
    count: usize,
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_data_frames(framed, cid, count).await;
    let term = StreamTestEnvelope::terminator(cid);
    if let Ok(encoded) = term.serialize_to_bytes() {
        let _ = framed.send(encoded).await;
    }
}
