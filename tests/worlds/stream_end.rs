//! Test world for verifying stream terminators and multi-packet lifecycle logs.
//!
//! Provides [`StreamEndWorld`] so cucumber scenarios can observe terminator
//! frames, closure reasons, and shutdown handling for streaming responses.
#![cfg(not(loom))]

use std::{mem, sync::Arc};

use async_stream::try_stream;
use cucumber::World;
use log::Level;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::{ConnectionActor, ConnectionChannels, test_support::ActorHarness},
    hooks::ProtocolHooks,
    response::FrameStream,
};
use wireframe_testing::{LoggerHandle, logger};

use super::{Terminator, TestResult, build_small_queues};

#[derive(Debug, Default, World)]
pub struct StreamEndWorld {
    frames: Vec<u8>,
    logs: Vec<(Level, String)>,
}

enum MultiPacketMode {
    Disconnect { send_frames: bool },
    Shutdown,
}

enum ActorMode {
    Stream,
    MultiPacket,
}

impl StreamEndWorld {
    fn prepare_test(&mut self) -> LoggerHandle {
        self.frames.clear();
        self.logs.clear();
        let mut logger = logger();
        logger.clear();
        logger
    }

    fn finalize_test(&mut self, logger: &mut LoggerHandle) { self.capture_logs(logger); }

    async fn run_actor_test(&mut self, mode: ActorMode) -> TestResult {
        let mut temp = StreamEndWorld::default();
        mem::swap(self, &mut temp);
        let mut logger = temp.prepare_test();

        let (queues, handle) = build_small_queues::<u8>()?;
        let shutdown = CancellationToken::new();
        let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));

        match mode {
            ActorMode::Stream => {
                let stream: FrameStream<u8> = Box::pin(try_stream! {
                    yield 1u8;
                    yield 2u8;
                });
                let mut actor = ConnectionActor::with_hooks(
                    ConnectionChannels::new(queues, handle),
                    Some(stream),
                    shutdown,
                    hooks,
                );
                actor
                    .run(&mut temp.frames)
                    .await
                    .map_err(|e| format!("actor run failed: {e:?}"))?;
            }
            ActorMode::MultiPacket => {
                let (tx, rx) = mpsc::channel(4);
                tx.send(1u8).await?;
                tx.send(2u8).await?;
                drop(tx);

                let mut actor = ConnectionActor::with_hooks(
                    ConnectionChannels::new(queues, handle),
                    None,
                    shutdown,
                    hooks,
                );
                actor.set_multi_packet(Some(rx));
                actor
                    .run(&mut temp.frames)
                    .await
                    .map_err(|e| format!("actor run failed: {e:?}"))?;
            }
        }

        temp.finalize_test(&mut logger);
        mem::swap(self, &mut temp);
        Ok(())
    }

    /// Run the connection actor and record emitted frames.
    ///
    /// # Errors
    /// Returns an error if the actor fails to run successfully.
    pub async fn process(&mut self) -> TestResult { self.run_actor_test(ActorMode::Stream).await }

    /// Run the connection actor with a multi-packet channel and record emitted frames.
    ///
    /// # Errors
    /// Returns an error if sending to the channel or running the actor fails.
    pub async fn process_multi(&mut self) -> TestResult {
        self.run_actor_test(ActorMode::MultiPacket).await
    }

    fn capture_logs(&mut self, logger: &mut LoggerHandle) {
        while let Some(record) = logger.pop() {
            self.logs.push((record.level(), record.args().to_string()));
        }
    }

    fn closure_log(&self) -> Option<&(Level, String)> {
        self.logs
            .iter()
            .rev()
            .find(|(_, message)| message.contains("multi-packet stream closed"))
    }

    fn run_multi_packet_harness(
        &mut self,
        mode: &MultiPacketMode,
        correlation_id: u64,
    ) -> TestResult {
        let mut temp = StreamEndWorld::default();
        mem::swap(self, &mut temp);
        let mut logger = temp.prepare_test();

        let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
        let mut harness = ActorHarness::new_with_state(hooks, false, true)?;
        let (tx, rx) = mpsc::channel(4);
        harness
            .actor_mut()
            .set_multi_packet_with_correlation(Some(rx), Some(correlation_id));
        match mode {
            MultiPacketMode::Disconnect { send_frames } => {
                if *send_frames {
                    tx.try_send(1u8)?;
                    tx.try_send(2u8)?;
                }
                drop(tx);
                logger.clear();
                while harness.try_drain_multi() {}
            }
            MultiPacketMode::Shutdown => {
                drop(tx);
                logger.clear();
                harness.start_shutdown();
            }
        }
        temp.frames.clone_from(&harness.out);

        temp.finalize_test(&mut logger);
        mem::swap(self, &mut temp);
        Ok(())
    }

    /// Simulate a disconnected multi-packet channel by dropping the sender before draining.
    ///
    /// # Errors
    /// Returns an error if creating the harness or sending frames fails.
    pub fn process_multi_disconnect(&mut self) -> TestResult {
        self.run_multi_packet_harness(&MultiPacketMode::Disconnect { send_frames: true }, 42)
    }

    /// Trigger shutdown handling on a multi-packet channel without emitting a terminator.
    ///
    /// # Errors
    /// Returns an error if creating the harness fails.
    pub fn process_multi_shutdown(&mut self) -> TestResult {
        self.run_multi_packet_harness(&MultiPacketMode::Shutdown, 77)
    }

    /// Verify that a terminator frame was appended to the stream.
    ///
    /// # Panics
    /// Panics if the expected terminator is missing.
    pub fn verify(&self) {
        assert_eq!(self.frames, vec![1, 2, 0]);
    }

    /// Verify that a multi-packet terminator frame was appended to the stream.
    ///
    /// # Panics
    /// Panics if the expected terminator is missing.
    pub fn verify_multi(&self) {
        assert_eq!(self.frames, vec![1, 2, 0]);
    }

    /// Verify that no terminator frame was emitted.
    ///
    /// # Panics
    /// Panics if a terminator frame is present.
    pub fn verify_no_multi(&self) {
        assert!(
            self.frames.iter().all(|&frame| frame != 0),
            "unexpected terminator frame present",
        );
    }

    /// Verify the logged multi-packet termination reason.
    ///
    /// # Errors
    /// Returns an error if the closure log is missing or contains unexpected
    /// details.
    pub fn verify_reason(&self, expected: &str) -> TestResult {
        let (level, message) = self
            .closure_log()
            .ok_or("multi-packet closure log missing")?;
        let expected_level = match expected {
            "disconnected" => Level::Warn,
            _ => Level::Info,
        };
        if *level != expected_level {
            return Err("unexpected log level for closure".into());
        }
        if !message.contains(&format!("reason={expected}")) {
            return Err("closure log missing reason detail".into());
        }
        Ok(())
    }
}
