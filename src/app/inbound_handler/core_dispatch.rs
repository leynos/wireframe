//! Step handling and frame dispatch for the inbound connection loop.
//!
//! [`ConnectionLoop`] gains one unit of work at a time, and these methods act
//! on that unit. They stay inherent methods because each one drives the loop's
//! own sink, reader, buffered inbound state and response actor, and live in
//! their own file so the loop's definition stays within the module size limit.

use std::io;

use futures::StreamExt;
use log::debug;

use super::{
    AsyncRead,
    AsyncWrite,
    ConnectionLoop,
    DecodeWith,
    DecodedItem,
    Dispatch,
    EncodeWith,
    Envelope,
    FrameCodec,
    FrameHandlingContext,
    FrameMetadata,
    Packet,
    PeerObservation,
    Serializer,
    StreamStep,
    handle_frame,
    handle_watching_peer,
    hold_or_end,
    observe_peer,
    resolve_read_item,
    settle_dispatch,
};

impl<S, E, F, W> ConnectionLoop<'_, S, E, F, W>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Act on one unit of work, reporting whether the connection continues.
    ///
    /// The push steps are handled in `core_pushes`; this module owns the frame
    /// path, where a decoder failure or a decoded frame is resolved and
    /// dispatched.
    pub(super) async fn handle_step(
        &mut self,
        step: StreamStep<F, DecodedItem<F::Frame>>,
    ) -> io::Result<bool> {
        match step {
            StreamStep::Pushes(pending) => self.write_pushes(pending).await,
            StreamStep::Idle => {
                debug!("push lanes closed; continuing to wait for the next frame");
                Ok(true)
            }
            StreamStep::Closed => {
                // Frames already queued are still delivered; anything pushed
                // afterwards is rejected because the actor closes both lanes.
                self.drain_pushes().await?;
                Ok(false)
            }
            // A failed or cancelled response leaves its stream without a
            // terminator, so a later response would be read as part of the
            // truncated stream. Stop instead.
            StreamStep::Frame(item) => Ok(self.dispatch(item).await? == Dispatch::Continue),
        }
    }

    /// Resolve one read item into a frame, then dispatch it.
    ///
    /// A decoder failure is resolved here rather than inside the dispatch
    /// steps: the policy can discard or quarantine it, which is a property of
    /// the read loop. The decoded frame is then handled while the peer is
    /// watched for a disconnect, and a buffered failure is reported if that
    /// handling ends the connection.
    async fn dispatch(&mut self, item: DecodedItem<F::Frame>) -> io::Result<Dispatch> {
        let Some(frame) =
            resolve_read_item(item, &mut self.inbound, &mut self.recovery, &self.shutdown).await?
        else {
            return Ok(Dispatch::Continue);
        };
        let (unobserved, cause) = observe_peer(&mut self.response_actor.actor, &self.peer_eof);
        let handling = handle_frame(
            &frame,
            FrameHandlingContext {
                sink: &mut self.sink,
                deser_failures: &mut self.deser_failures,
                routes: self.routes,
                serializer: self.serializer.as_ref(),
                codec: &self.codec,
                message_assembler: self.message_assembler,
                message_assembly: &mut self.inbound.message_assembly,
                pipeline: &mut self.inbound.pipeline,
                actor: &mut self.response_actor.actor,
            },
        );
        // A read made while the response is pending is buffered for the next
        // dispatch, unless it reports the end of the stream: that ends the
        // connection now, which is the only way a truncation is observed while
        // a response is still running.
        let recovery = &self.recovery;
        let mut watched = (&mut self.reader)
            .map(|item: io::Result<_>| item.and_then(|it| hold_or_end(it, recovery)));
        let dispatch = handle_watching_peer(
            handling,
            &mut watched,
            PeerObservation::new(&self.peer_eof, &unobserved, &cause),
            &mut self.inbound.pending,
        )
        .await;
        settle_dispatch(dispatch, &mut self.inbound, &self.recovery)
    }
}
