//! Demonstrates multi-packet responses using `Response::with_channel`.
//!
//! The example splits a transcript into multiple frames and streams them via a
//! bounded channel. Two background tasks send frames concurrently to showcase
//! how cloning the sender enables cooperative production while back-pressure
//! keeps senders in lock-step with the consumer.

use std::time::Duration;

use futures::TryStreamExt;
use tokio::time::sleep;
use wireframe::Response;

const TRANSCRIPT: &[&str] = &[
    "Client: HELLO",
    "Server: HELLO-ACK",
    "Client: BEGIN",
    "Server: PREPARED",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameKind {
    Chunk(usize),
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    kind: FrameKind,
    data: String,
}

impl Frame {
    fn chunk(index: usize, data: &str) -> Self {
        Self {
            kind: FrameKind::Chunk(index),
            data: data.to_owned(),
        }
    }

    fn summary(total_chunks: usize) -> Self {
        Self {
            kind: FrameKind::Summary,
            data: format!("{total_chunks} transcript entries sent"),
        }
    }
}

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
                // The connection dropped; stop work early.
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
            // The connection dropped; stop work early.
        }
    });

    response
}

#[tokio::main]
async fn main() {
    let response = multi_packet_response();
    let mut stream = response.into_stream();

    while let Some(frame) = stream
        .try_next()
        .await
        .expect("multi-packet stream should not fail")
    {
        match frame {
            Frame {
                kind: FrameKind::Chunk(index),
                data,
            } => println!("Chunk {index}: {data}"),
            Frame {
                kind: FrameKind::Summary,
                data,
            } => println!("Summary: {data}"),
        }
    }
}
