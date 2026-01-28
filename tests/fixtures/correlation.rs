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

// Import build_small_queues from parent module
use crate::build_small_queues;
/// Re-export `TestResult` from common for use in steps.
pub use crate::common::TestResult;

#[derive(Debug, Default)]
/// Test world capturing correlation expectations for frame emission.
pub struct CorrelationWorld {
    expected: Option<u64>,
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
        tx.send(Envelope::new(1, None, vec![1])).await?;
        tx.send(Envelope::new(1, Some(99), vec![2])).await?;
        drop(tx);

        let (queues, handle) = build_small_queues::<Envelope>()?;
        let shutdown = CancellationToken::new();
        let mut actor: ConnectionActor<Envelope, ()> =
            ConnectionActor::new(queues, handle, None, shutdown);
        actor.set_multi_packet_with_correlation(Some(rx), expected);
        actor
            .run(&mut self.frames)
            .await
            .map_err(|e| format!("actor run failed: {e:?}"))?;
        Ok(())
    }

    /// Verify that all received frames respect the configured correlation
    /// expectation.
    ///
    /// # Errors
    /// Returns an error if any frame violates the stored correlation
    /// expectation.
    pub fn verify(&self) -> TestResult {
        let ok = match self.expected {
            Some(cid) => self.frames.iter().all(|f| f.correlation_id() == Some(cid)),
            None => self.frames.iter().all(|f| f.correlation_id().is_none()),
        };

        if ok {
            return Ok(());
        }

        match self.expected {
            Some(cid) => Err(format!("frames missing expected correlation id {cid}").into()),
            None => Err("frames unexpectedly carried correlation id".into()),
        }
    }
}
