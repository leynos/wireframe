//! Demonstrates multi-packet responses using `Response::with_channel`.
//!
//! The example splits a transcript into multiple frames and streams them via a
//! bounded channel. Two background tasks send frames concurrently to showcase
//! how cloning the sender enables cooperative production while back-pressure
//! keeps senders in lock-step with the consumer.

use std::{
    fmt::{self, Display, Formatter},
    time::Duration,
};

use futures::TryStreamExt;
use tokio::time::sleep;
use tracing::info;
use wireframe::{WireframeError, response::Response};

/// Ordered application messages emitted before the final stream summary.
/// Keeping this fixture static makes the example's frame ordering reproducible.
const TRANSCRIPT: &[&str] = &[
    "Client: HELLO",
    "Server: HELLO-ACK",
    "Client: BEGIN",
    "Server: PREPARED",
];

#[derive(Debug, Clone, PartialEq, Eq)]
/// Distinguishes a transcript chunk from the terminal stream summary.
enum FrameKind {
    /// Identifies the zero-based transcript entry carried by a frame.
    Chunk(usize),
    /// Marks the final frame after every chunk producer has completed.
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One logical response item sent through the bounded response channel.
struct Frame {
    /// Identifies whether `data` is an ordered chunk or the terminal summary.
    kind: FrameKind,
    /// Human-readable transcript content carried by this response item.
    data: String,
}

impl Frame {
    /// Creates a chunk while preserving its position in the transcript.
    fn chunk(index: usize, data: &str) -> Self {
        Self {
            kind: FrameKind::Chunk(index),
            data: data.to_owned(),
        }
    }

    /// Creates the terminal item reporting how many chunks were attempted.
    fn summary(total_chunks: usize) -> Self {
        Self {
            kind: FrameKind::Summary,
            data: format!("{total_chunks} transcript entries sent"),
        }
    }
}

impl Display for Frame {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self.kind {
            FrameKind::Chunk(index) => write!(formatter, "Chunk {index}: {}", self.data),
            FrameKind::Summary => write!(formatter, "Summary: {}", self.data),
        }
    }
}

/// Starts concurrent chunk production and returns a bounded response stream.
/// The summary task waits for the chunk task, preserving ordering even though
/// both producers own senders; a dropped consumer causes each sender to stop.
fn multi_packet_response() -> Response<Frame> {
    // Capacity two keeps memory usage tight and amplifies the back-pressure
    // effect for demonstration purposes.
    let (sender, response) = Response::with_channel(2);

    let summary_sender = sender.clone();
    let chunk_task = tokio::spawn(async move {
        // Capture the join handle so the summary task can wait for completion
        // before sending its final frame.
        for (index, line) in TRANSCRIPT.iter().enumerate() {
            let frame = Frame::chunk(index, line);
            if sender.send(frame).await.is_err() {
                tracing::trace!("connection dropped, stopping chunk task early");
                break;
            }

            // The brief pause simulates I/O or computation between frames.
            sleep(Duration::from_millis(25)).await;
        }
    });

    tokio::spawn(async move {
        // Wait for all chunks to be sent before delivering the summary.
        let _ = chunk_task.await;
        let summary = Frame::summary(TRANSCRIPT.len());
        if summary_sender.send(summary).await.is_err() {
            tracing::trace!("connection dropped, summary not sent");
        }
    });

    response
}

/// Logs one response item using the example's display representation.
fn log_frame(frame: &Frame) {
    info!("{frame}");
}

/// Consumes the response until all producers close the channel.
/// Stream errors are propagated so the example demonstrates its normal error
/// boundary rather than silently truncating a multipart response.
async fn run() -> Result<(), WireframeError> {
    let _ = tracing_subscriber::fmt::try_init();
    let response = multi_packet_response();
    let mut stream = response.into_stream();
    while let Some(frame) = stream.try_next().await? {
        log_frame(&frame);
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime
        .block_on(run())
        .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    Ok(())
}
