//! `CorrelationWorld` fixture for rstest-bdd tests.
//!
//! Converted from Cucumber World to rstest fixture. The struct and its methods
//! remain largely unchanged; only the trait derivation and fixture function are
//! added.

use async_stream::try_stream;
use rstest::fixture;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::Envelope,
    connection::ConnectionActor,
    correlation::CorrelatableFrame,
    response::FrameStream,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

// Import build_small_queues from parent module
use crate::build_small_queues;

/// Multi-packet frames emitted by the scenarios, paired with their payloads.
///
/// The second frame carries its own identifier so the no-request-id scenario
/// can prove that frame keeps it rather than having it cleared.
const MULTI_PACKET_FRAMES: [(Option<u64>, u8); 2] = [(None, 1), (Some(99), 2)];

#[derive(Debug, Default)]
/// Test world capturing correlation expectations for frame emission.
pub struct CorrelationWorld {
    expected: Option<u64>,
    /// Identifiers the multi-packet channel supplied on each frame.
    own_ids: Vec<Option<u64>>,
    frames: Vec<Envelope>,
}

// rustfmt collapses simple fixtures into one line, which triggers unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn correlation_world() -> CorrelationWorld {
    CorrelationWorld::default()
}

impl CorrelationWorld {
    /// Record the correlation identifier expected on emitted frames.
    pub fn set_expected(&mut self, expected: Option<u64>) { self.expected = expected; }

    /// Return the correlation identifier configured for this scenario.
    #[must_use]
    pub fn expected(&self) -> Option<u64> { self.expected }

    /// Run the connection actor and collect frames for later verification.
    ///
    /// # Errors
    /// Returns an error if the expected correlation id is absent or if running
    /// the actor fails.
    pub async fn process(&mut self) -> TestResult {
        let cid = self
            .expected
            .ok_or("streaming scenario requires a correlation id")?;
        let stream: FrameStream<Envelope> = Box::pin(try_stream! {
            yield Envelope::new(1, Some(cid), vec![1]);
            yield Envelope::new(1, Some(cid), vec![2]);
        });
        let (queues, handle) = build_small_queues::<Envelope>()?;
        let shutdown = CancellationToken::new();
        let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
        actor
            .run(&mut self.frames)
            .await
            .map_err(|e| format!("actor run failed: {e:?}"))?;
        Ok(())
    }

    /// Run the connection actor for a multi-packet channel and collect frames.
    ///
    /// # Errors
    /// Returns an error if sending frames or running the actor fails.
    pub async fn process_multi(&mut self) -> TestResult {
        let expected = self.expected;
        let (tx, rx) = mpsc::channel(4);
        for (id, payload) in MULTI_PACKET_FRAMES {
            tx.send(Envelope::new(1, id, vec![payload])).await?;
        }
        self.own_ids = MULTI_PACKET_FRAMES.iter().map(|(id, _)| *id).collect();
        drop(tx);

        let (queues, handle) = build_small_queues::<Envelope>()?;
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<Envelope, wireframe::NoProtocolError> =
            ConnectionActor::new(queues, handle, None, shutdown);
        actor
            .set_multi_packet_with_correlation(Some(rx), expected)
            .map_err(|e| format!("set_multi_packet_with_correlation failed: {e}"))?;
        actor
            .run(&mut self.frames)
            .await
            .map_err(|e| format!("actor run failed: {e:?}"))?;
        Ok(())
    }

    /// Verify that all received frames respect the configured correlation
    /// expectation.
    ///
    /// A request correlation id must appear on every frame. Without one, each
    /// frame keeps the identifier its source supplied.
    ///
    /// # Errors
    /// Returns an error if any frame violates the stored correlation
    /// expectation.
    pub fn verify(&self) -> TestResult {
        match self.expected {
            Some(cid) if self.frames.iter().all(|f| f.correlation_id() == Some(cid)) => Ok(()),
            Some(cid) => Err(format!("frames missing expected correlation id {cid}").into()),
            None => self.verify_own_ids(),
        }
    }

    /// Verify that frames without a request id kept their own identifiers.
    ///
    /// # Errors
    /// Returns an error when the frame count differs from the channel's
    /// emission, or when any identifier was cleared or overwritten.
    fn verify_own_ids(&self) -> TestResult {
        let matched = self.frames.len() == self.own_ids.len()
            && self
                .frames
                .iter()
                .zip(&self.own_ids)
                .all(|(frame, id)| frame.correlation_id() == *id);
        if matched {
            return Ok(());
        }
        Err("frames did not keep their own correlation ids".into())
    }
}
