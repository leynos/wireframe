//! World state for correlation id behavioural tests.

use cucumber::World;
use futures::{stream, StreamExt};
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet},
    connection::ConnectionActor,
    push::PushQueues,
    response::WireframeError,
};

#[derive(Default, World)]
pub struct CorrelationWorld {
    pub(crate) correlation_id: u32,
    frames: Vec<Envelope>,
}

impl std::fmt::Debug for CorrelationWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CorrelationWorld")
            .field("correlation_id", &self.correlation_id)
            .field("frames_len", &self.frames.len())
            .finish()
    }
}

impl CorrelationWorld {
    pub fn set_id(&mut self, id: u32) {
        self.correlation_id = id;
    }

    /// Run the connection actor and capture emitted frames.
    ///
    /// # Panics
    /// Panics if the actor fails to run.
    pub async fn run_actor(&mut self) {
        let (queues, handle) = PushQueues::bounded(1, 1);
        let stream_frames = stream::iter(vec![
            Ok::<Envelope, ()>(Envelope::new(1, self.correlation_id, vec![1])),
            Ok::<Envelope, ()>(Envelope::new(1, self.correlation_id, vec![2])),
        ])
        .map(|r: Result<Envelope, ()>| r.map_err(WireframeError::from));
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<Envelope, ()> =
            ConnectionActor::new(queues, handle, Some(Box::pin(stream_frames)), shutdown);
        let mut out = Vec::new();
        actor.run(&mut out).await.expect("actor run failed");
        self.frames = out;
    }

    /// Assert that all captured frames carry the expected correlation id.
    ///
    /// # Panics
    /// Panics if any frame has a mismatched identifier.
    pub fn assert_ids(&self) {
        assert!(self
            .frames
            .iter()
            .all(|f| f.correlation_id() == self.correlation_id));
    }
}
