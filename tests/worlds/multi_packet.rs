#![cfg(not(loom))]
//! Test world for multi-packet channel scenarios.
//!
//! Provides [`MultiPacketWorld`] to verify message ordering, back-pressure
//! handling, and channel lifecycle in cucumber-based behaviour tests.

use cucumber::World;
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio_util::sync::CancellationToken;
use wireframe::{Response, connection::ConnectionActor};

use super::build_small_queues;

#[derive(Debug, Default, World)]
pub struct MultiPacketWorld {
    messages: Vec<u8>,
    overflow_error: bool,
}

impl MultiPacketWorld {
    async fn collect_frames_from(rx: mpsc::Receiver<u8>) -> Vec<u8> {
        let (queues, handle) = build_small_queues::<u8>();
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<_, ()> =
            ConnectionActor::new(queues, handle, None, shutdown);
        actor.set_multi_packet(Some(rx));

        let mut frames = Vec::new();
        actor.run(&mut frames).await.expect("actor run failed");
        frames
    }

    /// Helper method to process messages through a multi-packet response built
    /// via [`Response::with_channel`].
    ///
    /// # Panics
    /// Panics if spawning or joining the producer task fails.
    async fn process_messages(&mut self, messages: &[u8]) {
        let (sender, response): (mpsc::Sender<u8>, Response<u8, ()>) = Response::with_channel(4);
        let Response::MultiPacket(rx) = response else {
            panic!("helper did not return a MultiPacket response");
        };

        let payload = messages.to_vec();
        let producer = tokio::spawn(async move {
            for msg in payload {
                if sender.send(msg).await.is_err() {
                    return;
                }
            }
            drop(sender);
        });

        let frames = Self::collect_frames_from(rx).await;
        producer.await.expect("producer task panicked");
        self.messages = frames;
        self.overflow_error = false;
    }

    /// Send messages through a multi-packet response and record them.
    ///
    /// # Panics
    /// Panics if sending to the channel fails.
    pub async fn process(&mut self) { self.process_messages(&[1, 2, 3]).await; }

    /// Record zero messages from a closed channel.
    ///
    /// # Panics
    /// Does not panic.
    pub async fn process_empty(&mut self) { self.process_messages(&[]).await; }

    /// Attempt to send more messages than the channel can buffer at once.
    ///
    /// # Panics
    /// Panics if sending to the channel fails unexpectedly or the producer task panics.
    pub async fn process_overflow(&mut self) {
        let (sender, response): (mpsc::Sender<u8>, Response<u8, ()>) = Response::with_channel(1);
        let Response::MultiPacket(rx) = response else {
            panic!("helper did not return a MultiPacket response");
        };

        sender.try_send(1).expect("send initial frame");
        let overflow_error = matches!(sender.try_send(2), Err(TrySendError::Full(2)));

        let producer = tokio::spawn(async move {
            sender
                .send(2)
                .await
                .expect("send follow-up frame after draining");
            drop(sender);
        });

        let frames = Self::collect_frames_from(rx).await;
        producer.await.expect("producer task panicked");

        self.messages = frames;
        self.overflow_error = overflow_error;
    }

    /// Verify that no messages were received.
    ///
    /// # Panics
    /// Panics if any messages are present.
    pub fn verify_empty(&self) {
        assert!(self.messages.is_empty());
    }

    /// Verify messages were received in order.
    ///
    /// # Panics
    ///
    /// Panics if the messages are not in the expected order.
    pub fn verify(&self) {
        assert_eq!(self.messages, vec![1, 2, 3]);
    }

    /// Verify that the channel enforced back-pressure.
    ///
    /// # Panics
    /// Panics if no overflow occurred or if the expected messages are missing.
    pub fn verify_overflow(&self) {
        assert!(
            self.overflow_error,
            "expected overflow error when channel capacity was exceeded",
        );
        assert_eq!(self.messages, vec![1, 2]);
    }
}
