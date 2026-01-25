//! `MultiPacketWorld` fixture for rstest-bdd tests.
//!
//! Provides test fixtures to verify message ordering, back-pressure handling,
//! and channel lifecycle.

#![expect(unused_braces, reason = "rustfmt forces single-line fixture functions")]

use std::{error::Error, fmt};

use rstest::fixture;
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio_util::sync::CancellationToken;
use wireframe::{Response, connection::ConnectionActor};

use crate::build_small_queues;
// Re-export TestResult from common for use in steps
pub use crate::common::TestResult;

#[derive(Debug)]
struct WireframeRunError(wireframe::WireframeError);

impl fmt::Display for WireframeRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{:?}", self.0) }
}

impl Error for WireframeRunError {}

#[derive(Debug, Default)]
/// Test world exercising multi-packet channel behaviours and back-pressure.
pub struct MultiPacketWorld {
    messages: Vec<u8>,
    is_overflow_error: bool,
}

#[fixture]
pub fn multi_packet_world() -> MultiPacketWorld { MultiPacketWorld::default() }

impl MultiPacketWorld {
    async fn collect_frames_from(rx: mpsc::Receiver<u8>) -> TestResult<Vec<u8>> {
        let (queues, handle) = build_small_queues::<u8>()?;
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<_, ()> =
            ConnectionActor::new(queues, handle, None, shutdown);
        actor.set_multi_packet(Some(rx));

        let mut frames = Vec::new();
        actor
            .run(&mut frames)
            .await
            .map_err(WireframeRunError)
            .map_err(Box::<dyn std::error::Error + Send + Sync>::from)?;
        Ok(frames)
    }

    /// Send a single byte with back-pressure then close the channel.
    async fn send_with_backpressure(sender: mpsc::Sender<u8>, value: u8) -> TestResult<()> {
        sender.send(value).await?;
        drop(sender);
        Ok(())
    }

    /// Helper method to process messages through a multi-packet response built
    /// via [`Response::with_channel`].
    ///
    /// # Errors
    /// Returns an error if the response cannot be converted to a multi-packet
    /// stream or if producer tasks fail.
    async fn process_messages(&mut self, messages: &[u8]) -> TestResult {
        let (sender, response): (mpsc::Sender<u8>, Response<u8, ()>) = Response::with_channel(4);
        let Response::MultiPacket(rx) = response else {
            return Err("helper did not return a MultiPacket response".into());
        };

        let payload = messages.to_vec();
        let producer = tokio::spawn(Self::send_payload(sender, payload));

        let frames = Self::collect_frames_from(rx).await?;
        producer.await?;
        self.messages = frames;
        self.is_overflow_error = false;
        Ok(())
    }

    /// Send each byte to the channel, stopping silently if the receiver closes
    /// to simulate a producer completing without error when the consumer is
    /// gone.
    async fn send_payload(sender: mpsc::Sender<u8>, payload: Vec<u8>) {
        for msg in payload {
            if sender.send(msg).await.is_err() {
                return;
            }
        }
    }

    /// Send messages through a multi-packet response and record them.
    ///
    /// # Errors
    /// Returns an error if the response cannot be converted to a multi-packet
    /// stream or if producer tasks fail.
    pub async fn process(&mut self) -> TestResult { self.process_messages(&[1, 2, 3]).await }

    /// Record zero messages from a closed channel.
    ///
    /// # Errors
    /// Returns an error if the response cannot be converted to a multi-packet
    /// stream or if producer tasks fail.
    pub async fn process_empty(&mut self) -> TestResult { self.process_messages(&[]).await }

    /// Attempt to send more messages than the channel can buffer at once.
    ///
    /// # Errors
    /// Returns an error if sending to the channel fails unexpectedly or the
    /// producer task returns an error.
    pub async fn process_overflow(&mut self) -> TestResult {
        let (sender, response): (mpsc::Sender<u8>, Response<u8, ()>) = Response::with_channel(1);
        let Response::MultiPacket(rx) = response else {
            return Err("helper did not return a MultiPacket response".into());
        };

        sender.try_send(1)?;
        let overflow_error = matches!(sender.try_send(2), Err(TrySendError::Full(2)));

        let producer = tokio::spawn(Self::send_with_backpressure(sender, 2));

        let frames = Self::collect_frames_from(rx).await?;
        // Unwrap JoinError from await, then the task's Result
        producer.await??;

        self.messages = frames;
        self.is_overflow_error = overflow_error;
        Ok(())
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
            self.is_overflow_error,
            "expected overflow error when channel capacity was exceeded",
        );
        assert_eq!(self.messages, vec![1, 2]);
    }
}
