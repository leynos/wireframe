//! Transport test double and the owned resources a delivery test borrows.
//!
//! Every case in the parent module writes through the same serializer, codec,
//! and pipeline, and differs only in whether the sink paces its writes. Owning
//! those parts here keeps the delivery assertions in each test free of
//! transport plumbing, while a test borrows them as a [`PushTransport`] for the
//! single call it makes.

use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use futures::Sink;
use rstest::fixture;

use super::super::PushTransport;
use crate::{
    app::{PushWriteDiagnostics, codec_driver::FramePipeline},
    codec::LengthDelimitedFrameCodec,
    serializer::BincodeSerializer,
};

/// Maximum frame length used by the in-memory codec.
const MAX_FRAME_LENGTH: usize = 2048;
/// Pause applied before each sink write after the first.
///
/// Below the idle grace, so every write is accepted in time, but long enough
/// that three frames outlast one grace in total.
const PACING: Duration = Duration::from_secs(20);

/// Sink that records what it wrote and can pace later writes.
///
/// The first write is accepted at once; every later one waits a full pacing
/// interval when one is configured. That models a peer that keeps reading
/// slowly, and makes the batch outlast one idle grace without any single wait
/// reaching it. A sink built with [`PacedSink::immediate`] configures no pause
/// and accepts every write at once.
pub(super) struct PacedSink {
    /// Frames accepted so far, in write order.
    accepted: Arc<Mutex<Vec<Bytes>>>,
    /// Pause owed to each write after the first, or `None` to accept at once.
    pacing: Option<Duration>,
    /// Pause owed to the next write, armed once a write has happened.
    delay: Option<Pin<Box<tokio::time::Sleep>>>,
    /// Whether every write reports itself unready, so none is ever accepted.
    never_ready: bool,
}

impl PacedSink {
    /// Share a sink that pauses between writes with the test.
    fn new() -> (Self, Arc<Mutex<Vec<Bytes>>>) { Self::with_pacing(Some(PACING)) }

    /// Share a sink that accepts every write at once with the test.
    fn immediate() -> (Self, Arc<Mutex<Vec<Bytes>>>) { Self::with_pacing(None) }

    /// Share a sink whose writes stay pending, so a write never completes.
    #[cfg(feature = "metrics")]
    fn blocked() -> (Self, Arc<Mutex<Vec<Bytes>>>) {
        let (mut sink, accepted) = Self::with_pacing(None);
        sink.never_ready = true;
        (sink, accepted)
    }

    /// Build a sink and share its record of accepted frames with the test.
    fn with_pacing(pacing: Option<Duration>) -> (Self, Arc<Mutex<Vec<Bytes>>>) {
        let accepted = Arc::new(Mutex::new(Vec::new()));
        let sink = Self {
            accepted: Arc::clone(&accepted),
            pacing,
            delay: None,
            never_ready: false,
        };
        (sink, accepted)
    }

    /// Record one accepted frame and arm the next write's pause.
    fn accept(&mut self, frame: Bytes) {
        self.accepted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frame);
        self.delay = self
            .pacing
            .map(|pacing| Box::pin(tokio::time::sleep(pacing)));
    }
}

impl Sink<Bytes> for PacedSink {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.never_ready {
            return Poll::Pending;
        }
        let is_ready = self
            .delay
            .as_mut()
            .is_none_or(|delay| matches!(delay.as_mut().poll(cx), Poll::Ready(())));
        if !is_ready {
            return Poll::Pending;
        }
        self.delay = None;
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, frame: Bytes) -> Result<(), Self::Error> {
        self.accept(frame);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Owned transport resources a delivery test borrows for one push call.
///
/// The sink and its record of accepted frames stay together, so a test reads
/// what was written only after the borrow of the transport has ended.
pub(super) struct TransportRig {
    /// Serializer shared with the response path.
    serializer: BincodeSerializer,
    /// Codec used to configure the framed transport.
    codec: LengthDelimitedFrameCodec,
    /// Outbound pipeline supplying the per-connection `before_send` hook.
    pipeline: FramePipeline<Bytes>,
    /// Write half of the framed transport.
    sink: PacedSink,
    /// Frames the sink accepted, in write order.
    accepted: Arc<Mutex<Vec<Bytes>>>,
}

impl TransportRig {
    /// Pair a sink with the transport parts every case shares.
    fn new(sink: PacedSink, accepted: Arc<Mutex<Vec<Bytes>>>) -> Self {
        Self {
            serializer: BincodeSerializer,
            codec: LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH),
            pipeline: FramePipeline::new(None),
            sink,
            accepted,
        }
    }

    /// Borrow the owned values as the transport one push call writes through.
    ///
    /// The borrow ends with the call that takes it, so a test can write first
    /// and then read what the sink accepted.
    pub(super) fn transport(
        &mut self,
    ) -> PushTransport<'_, BincodeSerializer, PacedSink, LengthDelimitedFrameCodec> {
        PushTransport {
            serializer: &self.serializer,
            codec: &self.codec,
            sink: &mut self.sink,
            pipeline: &mut self.pipeline,
            diagnostics: PushWriteDiagnostics::new(None, None),
        }
    }

    /// Snapshot the frames the sink accepted.
    pub(super) fn accepted_frames(&self) -> Vec<Bytes> {
        self.accepted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A rig whose sink pauses between writes, so a batch outlasts one grace.
#[fixture]
pub(super) fn paced_transport() -> TransportRig {
    let (sink, accepted) = PacedSink::new();
    TransportRig::new(sink, accepted)
}

/// A rig whose sink accepts every write at once.
#[fixture]
pub(super) fn immediate_transport() -> TransportRig {
    let (sink, accepted) = PacedSink::immediate();
    TransportRig::new(sink, accepted)
}

/// A rig whose sink never accepts a write, so a pending write is abandoned.
#[cfg(feature = "metrics")]
#[fixture]
pub(super) fn blocked_transport() -> TransportRig {
    let (sink, accepted) = PacedSink::blocked();
    TransportRig::new(sink, accepted)
}
