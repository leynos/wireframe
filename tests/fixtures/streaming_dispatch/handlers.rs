//! Route handlers driven by the streaming dispatch scenarios.
//!
//! Each handler reports what it observed through the scenario's event channel
//! and answers on the ordinary reply path, so a scenario can assert delivery
//! order and reply framing from outside the connection.

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::{Semaphore, mpsc};
use wireframe::{
    app::{Envelope, Handler, StreamingHandler},
    request::{RequestBodyStream, RequestParts},
};

use super::{DispatchEvent, HandlerMode};

/// Reply payload returned once a streaming body has been consumed.
const STREAM_REPLY: &[u8] = b"stream-reply";
/// Reply payload returned by a handler that abandons its body.
const ABANDONED_REPLY: &[u8] = b"abandoned";

/// Build the streaming route handler for the given mode.
pub(super) fn build_streaming_handler(
    mode: HandlerMode,
    gate: Arc<Semaphore>,
    events: mpsc::UnboundedSender<DispatchEvent>,
) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        let handler_gate = Arc::clone(&gate);
        let handler_events = events.clone();
        Box::pin(async move {
            if mode == HandlerMode::Abandon {
                return Some(Bytes::from_static(ABANDONED_REPLY));
            }
            if mode == HandlerMode::Hold {
                let _permit = handler_gate.acquire().await;
            }
            consume_body(body, &handler_events).await;
            Some(Bytes::from_static(STREAM_REPLY))
        })
    })
}

/// Read a body to its end, reporting each chunk and how it ended.
///
/// The first error terminates the body, so a completed body and an errored
/// body are mutually exclusive observations.
async fn consume_body(mut body: RequestBodyStream, events: &mpsc::UnboundedSender<DispatchEvent>) {
    while let Some(item) = body.next().await {
        match item {
            Ok(chunk) => {
                let _sent = events.send(DispatchEvent::Chunk {
                    body: chunk.to_vec(),
                });
            }
            Err(error) => {
                let _sent = events.send(DispatchEvent::Error { kind: error.kind() });
                return;
            }
        }
    }
    let _sent = events.send(DispatchEvent::Completed);
}

/// Build the buffered route handler, which reports its payload.
pub(super) fn build_buffered_handler(
    events: mpsc::UnboundedSender<DispatchEvent>,
) -> Handler<Envelope> {
    Arc::new(move |envelope: &Envelope| {
        let handler_events = events.clone();
        let body = envelope.payload_bytes().to_vec();
        Box::pin(async move {
            let _sent = handler_events.send(DispatchEvent::Buffered { body });
        })
    })
}
