//! World state for correlation id behavioural tests.

use cucumber::World;
use futures::{StreamExt, stream};
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

    /// Run the connection actor with supplied frames and capture output.
    ///
    /// # Panics
    /// Panics if the actor fails to run.
    pub async fn run_actor_with_frames(&mut self, frames: Vec<Envelope>, queue_capacity: usize) {
        let (queues, handle) = PushQueues::bounded(queue_capacity, queue_capacity);
        let stream_frames = stream::iter(frames.into_iter().map(Ok::<Envelope, ()>))
            .map(|r| r.map_err(WireframeError::from));
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<Envelope, ()> =
            ConnectionActor::new(queues, handle, Some(Box::pin(stream_frames)), shutdown);
        let mut out = Vec::new();
        actor.run(&mut out).await.expect("actor run failed");
        self.frames = out;
    }

    /// Run the connection actor using two sequential frames with the current
    /// correlation identifier.
    #[allow(dead_code)]
    pub async fn run_actor(&mut self) {
        self.run_actor_with_frames(
            vec![
                Envelope::new(1, self.correlation_id, vec![1]),
                Envelope::new(1, self.correlation_id, vec![2]),
            ],
            1,
        )
        .await;
    }

    /// Assert that all captured frames carry the expected correlation id.
    ///
    /// # Panics
    /// Panics if any frame has a mismatched identifier.
    pub fn assert_ids(&self) {
        assert!(
            self.frames
                .iter()
                .all(|f| f.correlation_id() == self.correlation_id)
        );
    }
}
