//! Handles and per-chunk data shared by the streaming dispatch modules.
//!
//! The connection loop, the dispatch slot, and the parking queue each need the
//! same four mutable connection handles to answer a streaming handler, plus the
//! read timeout that bounds teardown. Grouping them here keeps every helper
//! within the repository's argument limit, and makes it explicit that a reply
//! travels the same pipeline whichever path produced it.

use std::{collections::VecDeque, time::Duration};

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use super::{
    super::{codec_driver::FramePipeline, combined_codec::ConnectionCodec, envelope::Packet},
    stream_dispatch::DispatchContext,
    stream_parked::ParkedMessage,
    stream_slot::ConnectionStreams,
};
use crate::{
    codec::FrameCodec,
    message_assembler::{MessageAssemblyState, MessageKey},
    request::RequestParts,
    serializer::Serializer,
};

/// Mutable connection handles needed to answer a streaming handler.
///
/// Borrow them for one call with [`DispatchHandles::reborrow`] when the caller
/// must keep using the bundle afterwards.
pub(super) struct DispatchHandles<'a, S, E, F, W>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Connection-local assembly state.
    pub(super) assembly: &'a mut Option<MessageAssemblyState>,
    /// Outbound pipeline shared by every reply on the connection.
    pub(super) pipeline: &'a mut FramePipeline<F::Frame>,
    /// Framed connection the reply is written to.
    pub(super) framed: &'a mut Framed<W, ConnectionCodec<F>>,
    /// Immutable connection inputs.
    pub(super) ctx: &'a DispatchContext<'a, S, E, F>,
    /// Maximum wait for a handler or a body send while teardown runs.
    pub(super) wait: Duration,
}

impl<S, E, F, W> DispatchHandles<'_, S, E, F, W>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Reborrow every handle for a single call, leaving this bundle usable.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut handles = DispatchHandles { .. };
    /// slot.close_active(None, handles.reborrow()).await?;
    /// // `handles` is still live here.
    /// ```
    pub(super) const fn reborrow(&mut self) -> DispatchHandles<'_, S, E, F, W> {
        DispatchHandles {
            assembly: &mut *self.assembly,
            pipeline: &mut *self.pipeline,
            framed: &mut *self.framed,
            ctx: self.ctx,
            wait: self.wait,
        }
    }
}

/// Bytes retained elsewhere, and the aggregate limit a chunk must fit within.
#[derive(Clone, Copy, Debug)]
pub(super) struct ChunkBudget {
    /// Bytes retained by assembly, fragmentation, and parked messages.
    pub(super) other_retained: usize,
    /// Aggregate byte limit the queued body must stay within.
    pub(super) limit: usize,
}

/// The first body chunk of a streaming request.
#[derive(Debug)]
pub(super) struct FirstChunk {
    /// Assembly key, absent when no assembler participates.
    pub(super) key: Option<MessageKey>,
    /// Routing metadata captured from the first frame.
    pub(super) parts: RequestParts,
    /// First body chunk.
    pub(super) chunk: Bytes,
    /// Whether this chunk also completes the message.
    pub(super) is_last: bool,
}

/// A continuation chunk offered to the live body.
#[derive(Debug)]
pub(super) struct ChunkOffer {
    /// Message key the chunk belongs to.
    pub(super) key: MessageKey,
    /// Body bytes carried by the frame.
    pub(super) chunk: Bytes,
    /// Whether this chunk completes the message.
    pub(super) is_last: bool,
}
/// Mutable state the connection loop owns across frames.
pub(super) struct LoopState {
    /// The one live streaming exchange, if any.
    pub(super) streams: ConnectionStreams,
    /// Complete messages waiting for the dispatch slot.
    pub(super) parked: VecDeque<ParkedMessage>,
    /// Consecutive malformed-frame count.
    pub(super) deser_failures: u32,
}

impl LoopState {
    /// Create loop state for a fresh connection.
    pub(super) fn new() -> Self {
        Self {
            streams: ConnectionStreams::new(),
            parked: VecDeque::new(),
            deser_failures: 0,
        }
    }
}
