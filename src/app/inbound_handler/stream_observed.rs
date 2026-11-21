//! Peer-observed dispatch of the actions that produce a reply.
//!
//! A reply may stream for an unbounded time, so waiting for one before reading
//! the next inbound frame would hide a peer that has already gone away: the
//! connection task, its gauge guard, and the server drain would then stay
//! pending forever. Every action that writes, drains, or closes a reply
//! therefore polls the read half alongside itself.
//!
//! These actions live apart from [`super::stream_dispatch`] because the loop's
//! state machine and the actions it schedules together outgrew the
//! repository's module limit.

use std::io;

use bytes::Bytes;
use futures::{Sink, Stream};

use super::{
    super::{
        envelope::{Envelope, Packet},
        frame_handling::Dispatch,
    },
    actor::observe_peer,
    peer_watch::{PeerObservation, handle_watching_peer},
    stream_actions::{drain_parked, fail_body_ending_with},
    stream_dispatch::Connection,
    stream_frame::dispatch_frame,
    stream_handles::{ChunkBudget, DispatchHandles, LoopState},
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    message_assembler::StreamingAssemblyId,
    request::QueuedBytes,
    serializer::Serializer,
};

/// One reply-producing action that must not hide a peer that went away.
pub(super) enum ObservedAction<'a, T> {
    /// Dispatch one inbound frame.
    Frame(&'a T),
    /// Answer the messages parked behind a live request body.
    Drain,
    /// Forward the reply of a handler that resolved while the loop waited.
    Handler(Option<Bytes>),
    /// Fail the live body, forwarding whatever reply it still produced.
    FailBody(io::Error),
    /// Fail the bodies whose assemblies expired, forwarding their replies.
    Expire(Vec<StreamingAssemblyId>),
}

impl<S, E, R, K, F> Connection<'_, S, E, R, K, F>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    E: Packet,
    R: Stream<Item = io::Result<F::Frame>> + Unpin,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Run one reply-producing action while watching the peer's read half.
    ///
    /// Every action here writes, drains, or closes a reply that may stream for
    /// an unbounded time, so each polls the reader alongside itself. Without
    /// that, a peer that half-closed is never observed, the actor's bounded
    /// peer-close drain is never armed, and a producer that stops yielding
    /// keeps the connection alive with nobody left to answer.
    pub(super) async fn dispatch_observed(
        &mut self,
        state: &mut LoopState,
        queued: QueuedBytes,
        action: ObservedAction<'_, F::Frame>,
    ) -> io::Result<Dispatch> {
        // Built from field borrows rather than through `handles`, because the
        // reader and its buffered frame must stay reachable for the action.
        let mut handles = DispatchHandles {
            assembly: &mut self.inbound.message_assembly,
            pipeline: &mut self.inbound.pipeline,
            sink: &mut *self.sink,
            actor: &mut *self.actor,
            queued,
            ctx: self.ctx,
            wait: self.wait,
        };
        let (unobserved, cause) = observe_peer(handles.actor, self.peer_eof);
        let observation = PeerObservation::new(self.peer_eof, &unobserved, &cause);
        let pending = &mut self.inbound.pending;
        match action {
            ObservedAction::Frame(frame) => {
                handle_watching_peer(
                    dispatch_frame(frame, state, handles),
                    self.reader,
                    observation,
                    pending,
                )
                .await
            }
            ObservedAction::Drain => {
                handle_watching_peer(
                    drain_parked(state, &mut handles),
                    self.reader,
                    observation,
                    pending,
                )
                .await
            }
            ObservedAction::Handler(outcome) => {
                handle_watching_peer(
                    state.streams.finish_active(outcome, handles.reborrow()),
                    self.reader,
                    observation,
                    pending,
                )
                .await
            }
            ObservedAction::FailBody(reason) => {
                handle_watching_peer(
                    fail_body_ending_with(state, &mut handles, reason),
                    self.reader,
                    observation,
                    pending,
                )
                .await
            }
            ObservedAction::Expire(ids) => {
                let budget = ChunkBudget::measure(&handles, &state.parked);
                handle_watching_peer(
                    state.streams.expire(&ids, budget, handles.reborrow()),
                    self.reader,
                    observation,
                    pending,
                )
                .await
            }
        }
    }
}
