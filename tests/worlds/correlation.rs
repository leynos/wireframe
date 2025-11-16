#![cfg(not(loom))]
//! Test world driving correlation ID verification scenarios.

use async_stream::try_stream;
use cucumber::World;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet},
    connection::ConnectionActor,
    response::FrameStream,
};

use super::build_small_queues;

#[derive(Debug, Default, World)]
pub struct CorrelationWorld {
    expected: Option<u64>,
    frames: Vec<Envelope>,
}

impl CorrelationWorld {
    pub fn set_expected(&mut self, expected: Option<u64>) { self.expected = expected; }

    #[must_use]
    pub fn expected(&self) -> Option<u64> { self.expected }

    /// Run the connection actor and collect frames for later verification.
    ///
    /// # Panics
    /// Panics if the actor fails to run successfully.
    pub async fn process(&mut self) {
        let cid = self
            .expected
            .expect("streaming scenario requires a correlation id");
        let stream: FrameStream<Envelope> = Box::pin(try_stream! {
            yield Envelope::new(1, Some(cid), vec![1]);
            yield Envelope::new(1, Some(cid), vec![2]);
        });
        let (queues, handle) = build_small_queues::<Envelope>();
        let shutdown = CancellationToken::new();
        let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
        actor.run(&mut self.frames).await.expect("actor run failed");
    }

    /// Run the connection actor for a multi-packet channel and collect frames.
    ///
    /// # Panics
    /// Panics if sending to the channel or running the actor fails.
    pub async fn process_multi(&mut self) {
        let expected = self.expected;
        let (tx, rx) = mpsc::channel(4);
        tx.send(Envelope::new(1, None, vec![1]))
            .await
            .expect("send frame");
        tx.send(Envelope::new(1, Some(99), vec![2]))
            .await
            .expect("send frame");
        drop(tx);

        let (queues, handle) = build_small_queues::<Envelope>();
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<Envelope, ()> =
            ConnectionActor::new(queues, handle, None, shutdown);
        actor.set_multi_packet_with_correlation(Some(rx), expected);
        actor.run(&mut self.frames).await.expect("actor run failed");
    }

    /// Verify that all received frames respect the configured correlation expectation.
    ///
    /// # Panics
    /// Panics if any frame violates the stored correlation expectation.
    pub fn verify(&self) {
        match self.expected {
            Some(cid) => {
                assert!(self.frames.iter().all(|f| f.correlation_id() == Some(cid)));
            }
            None => {
                assert!(self.frames.iter().all(|f| f.correlation_id().is_none()));
            }
        }
    }
}
