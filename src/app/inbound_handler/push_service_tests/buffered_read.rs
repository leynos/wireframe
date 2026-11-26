//! One buffered item is the whole contract for a read made during a write.
//!
//! A push write keeps polling the read half so a peer that half-closed while
//! the transport stalled is still observed. The item it reads is buffered for
//! the next dispatch, in a slot that holds exactly one item: a second read
//! would overwrite the first. That matters most when the first item is a
//! decoder failure, because the overwritten failure never reaches the recovery
//! policy, `on_error`, or the codec-error counter.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::Stream;
use tokio_util::sync::CancellationToken;

use super::{
    PushBatch,
    READ_TIMEOUT_MS,
    transport::{TransportRig, blocked_transport},
};
use crate::{
    app::{
        codec_driver::FramePipeline,
        inbound_handler::{
            frames::{InboundExpiry, InboundFrames},
            push_write::write_frame,
        },
        recovery_codec::{DecodeFailure, DecodeOrigin, DecodedItem},
    },
    codec::{CodecError, LengthDelimitedFrameCodec, ProtocolError},
};

/// Decoder failure the buffered item carries, as the recovery-aware decoder
/// reports it.
fn buffered_failure() -> DecodedItem<Bytes> {
    let error = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 9 });
    DecodedItem::Failure(DecodeFailure::new(error.into(), DecodeOrigin::Decode, 4))
}

/// Reader that yields its items and then cancels the connection's shutdown.
///
/// The sink never accepts the write, so the read loop is the only thing that
/// can end it. Cancelling shutdown after the last item bounds the test whether
/// or not the loop reads past the first item; a regression then fails on its
/// assertion instead of hanging on a write that never completes.
struct ItemsThenStop<'a> {
    /// Items still to yield, in order.
    items: std::vec::IntoIter<io::Result<DecodedItem<Bytes>>>,
    /// Token cancelled once the last item has been yielded.
    shutdown: &'a CancellationToken,
}

impl Stream for ItemsThenStop<'_> {
    type Item = io::Result<DecodedItem<Bytes>>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let next = self.items.next();
        if self.items.len() == 0 {
            self.shutdown.cancel();
        }
        Poll::Ready(next)
    }
}

/// Buffered state whose pipeline holds no frames and whose slot is empty.
///
/// The recovery-aware reader yields items, not frames, so the buffered slot
/// holds a `DecodedItem` even though the pipeline still buffers frames.
fn empty_inbound() -> InboundFrames<DecodedItem<Bytes>, Bytes> {
    InboundFrames {
        pending: None,
        pipeline: FramePipeline::new(None),
        message_assembly: None,
        read_deadline: None,
        evicted_streams: Vec::new(),
    }
}

/// Reader that yields the buffered failure and then one valid frame.
fn failure_then_frame(shutdown: &CancellationToken) -> ItemsThenStop<'_> {
    ItemsThenStop {
        items: vec![
            Ok(buffered_failure()),
            Ok(DecodedItem::Frame(Bytes::from_static(b"later frame"))),
        ]
        .into_iter(),
        shutdown,
    }
}

/// Write one frame that the transport never accepts, bounded by the grace.
///
/// The peer is still open, so the fast path polls the reader alongside the
/// pending write. The reader cancels shutdown after its last item, which is
/// what ends the write.
async fn write_stalled(
    harness: &mut TransportRig,
    batch: &mut PushBatch<'_, LengthDelimitedFrameCodec, ItemsThenStop<'_>, DecodedItem<Bytes>>,
) -> crate::app::inbound_handler::push_write::FrameWrite {
    tokio::time::timeout(
        crate::connection::PEER_CLOSED_RESPONSE_GRACE * 2,
        write_frame(
            &mut harness.transport(),
            Bytes::from_static(b"stalled"),
            batch,
            false,
        ),
    )
    .await
    .expect("the write must be bounded")
    .expect("a shutdown abandonment is not an error")
}

/// A stalled push write keeps the first item it read and reads no further.
///
/// Both items are ready in turn, which is what a decoder failure followed by a
/// valid frame looks like to the read loop. The buffered slot must still hold
/// the failure once the write ends.
#[rstest::rstest]
#[tokio::test(start_paused = true)]
async fn a_stalled_push_write_keeps_the_first_item_it_read(
    #[from(blocked_transport)] mut harness: TransportRig,
) {
    let shutdown = CancellationToken::new();
    let peer_eof = CancellationToken::new();
    let mut inbound = empty_inbound();
    let mut reader = failure_then_frame(&shutdown);
    let mut evicted = Vec::new();
    let mut batch = PushBatch {
        frames: Vec::new(),
        priority: None,
        reader: &mut reader,
        pending_frame: &mut inbound.pending,
        expiry: InboundExpiry::new(
            &mut inbound.message_assembly,
            &mut inbound.read_deadline,
            READ_TIMEOUT_MS,
            &mut evicted,
        ),
        peer_eof: &peer_eof,
        shutdown: &shutdown,
    };

    let written = write_stalled(&mut harness, &mut batch).await;
    drop(batch);

    assert!(!written.may_continue(), "the stop must end the write");
    assert!(
        !written.peer_closed(),
        "the peer never closed its write half in this case"
    );
    match inbound.pending {
        Some(DecodedItem::Failure(failure)) => {
            let (_, origin, consumed) = failure.into_parts();
            assert_eq!(
                origin,
                DecodeOrigin::Decode,
                "the buffered item must still be the decoder failure"
            );
            assert_eq!(
                consumed, 4,
                "the failure must keep its own consumed-byte count"
            );
        }
        Some(DecodedItem::Frame(frame)) => {
            panic!("a later frame overwrote the buffered failure: {frame:?}");
        }
        None => panic!("the read item must be preserved for recovery"),
    }
}
