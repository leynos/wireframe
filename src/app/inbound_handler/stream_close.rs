//! Terminal paths that free the connection's streaming slot.
//!
//! Every way a body can end — the handler answering on its own, a frame-layer
//! failure, an expired assembly — has to release the same things: the handler
//! future, the queued chunk, and the assembly's discard state. The paths live
//! apart from [`super::stream_slot`] because the slot keeps its byte account,
//! its admission, and this teardown logic together, which outgrew the
//! repository's module limit. Splitting them keeps the slot's steady-state
//! bookkeeping readable while each teardown still ends in one place.

use std::io;

use bytes::Bytes;
use futures::Sink;

use super::{
    super::{
        envelope::{Envelope, Packet},
        frame_handling::Dispatch,
    },
    stream_diagnostics,
    stream_handles::{ChunkBudget, DispatchHandles},
    stream_parked::forward_reply,
    stream_slot::{BodyFailure, ConnectionStreams},
    stream_terminal::{
        BodyClose,
        FinalDelivery,
        HandlerWait,
        await_handler,
        body_close,
        deliver_final_chunk_then_await,
        deliver_then_await,
    },
};
use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    message_assembler::{MessageAssemblyState, StreamingAssemblyId},
    serializer::Serializer,
};

impl ConnectionStreams {
    /// Finish the exchange after the handler resolved on its own.
    pub(super) async fn finish_active<S, E, K, F>(
        &mut self,
        outcome: Option<Bytes>,
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        let Some(mut active) = self.active.take() else {
            return Ok(Dispatch::Continue);
        };
        self.forget_stall();
        // Cancelling the admission is safe: a reserved permit that never sent
        // releases itself, and a queued chunk is dropped with its guard. The
        // body is counted before the reply is written, because the handler may
        // still answer while nothing will read the bytes still to come.
        active.record_unfinished();
        discard_streaming(active.assembly, handles.assembly);
        forward_reply(active.route_id, active.correlation_id, outcome, handles).await
    }

    /// Fail the live body, await its handler, and forward any reply.
    pub(super) async fn close_active<S, E, K, F>(
        &mut self,
        terminal: Option<io::Error>,
        budget: ChunkBudget,
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        let Some(mut active) = self.active.take() else {
            return Ok(Dispatch::Continue);
        };
        self.forget_stall();
        let pending = active.pending.take();
        let open_sender = active.sender.take();
        discard_streaming(active.assembly, handles.assembly);
        let identity = active.identity();
        let wait = match body_close(pending, terminal, open_sender, identity) {
            BodyClose::FinalChunk(final_chunk) => {
                let delivery = FinalDelivery {
                    identity,
                    pending: final_chunk,
                    budget,
                };
                deliver_final_chunk_then_await(handles.wait, delivery, &mut active.handler).await
            }
            BodyClose::Fail { failure, sender } => {
                deliver_then_await(handles.wait, failure, sender, &mut active.handler).await
            }
            BodyClose::AwaitReply => await_handler(handles.wait, &mut active.handler).await,
        };
        let reply = match wait {
            HandlerWait::Replied(reply) => reply,
            HandlerWait::Deadline => {
                // The handler outlived the wait and its reply is abandoned.
                // Recording it keeps a hung handler distinguishable from one
                // that deliberately chose not to reply.
                stream_diagnostics::handler_deadline_exceeded(active.route_id);
                None
            }
        };
        forward_reply(active.route_id, active.correlation_id, reply, handles).await
    }

    /// Fail the body for an expired streaming assembly.
    ///
    /// The evicted assemblies are matched by identity, not by key: a
    /// replacement may already have reused the key, and it must keep its
    /// own body.
    pub(super) async fn expire<S, E, K, F>(
        &mut self,
        ids: &[StreamingAssemblyId],
        budget: ChunkBudget,
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        if !self.owns_assembly(ids) {
            return Ok(Dispatch::Continue);
        }
        self.close_active(
            Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "request body assembly expired before completion",
            )),
            budget,
            handles,
        )
        .await
    }

    /// Fail the body for a terminal assembly rejection.
    pub(super) async fn fail<S, E, K, F>(
        &mut self,
        failure: BodyFailure,
        budget: ChunkBudget,
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        let BodyFailure { key, error } = failure;
        if !self.is_key_live(std::slice::from_ref(&key)) {
            stream_diagnostics::no_live_handler(key);
            return Ok(Dispatch::Continue);
        }
        self.close_active(Some(error), budget, handles).await
    }
}

/// Switch one streaming assembly to discard mode if state is present.
///
/// The exchange names the assembly it owns, so an assembly that reused the key
/// after this one expired keeps receiving its frames.
fn discard_streaming(id: Option<StreamingAssemblyId>, assembly: &mut Option<MessageAssemblyState>) {
    if let (Some(owned), Some(assembly_state)) = (id, assembly.as_mut()) {
        assembly_state.discard_streaming_if(owned);
    }
}
