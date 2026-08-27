//! Demonstrates generating `Response::Stream` values using `async-stream`.
//!
//! The `stream_response` function yields five sequential frames using
//! `async_stream::try_stream`. It returns a `Response` that can be
//! consumed by a `ConnectionActor`.

use std::io;

use async_stream::try_stream;
use futures::StreamExt;
use tracing::info;
use wireframe::response::Response;

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
/// A small serializable payload used to make stream ordering visible.
///
/// The value is deliberately just a sequence number: the example can focus on
/// the producer yielding frames and the consumer awaiting them in order rather
/// than on application-specific message handling.
struct Frame(u32);

/// Builds a response whose frames are produced lazily by an asynchronous
/// stream.
///
/// The producer yields five sequence-numbered frames and then ends. Pinning
/// the stream in the response matches the ownership contract expected by a
/// `ConnectionActor`: the actor owns polling while the response remains a
/// single value that can be returned from a handler.
fn stream_response() -> Response<Frame> {
    let frames = try_stream! {
        for n in 0..5u32 {
            yield Frame(n);
        }
    };
    Response::Stream(Box::pin(frames))
}

/// Runs the stream consumer and logs each frame as it arrives.
///
/// Awaiting `next` one item at a time demonstrates that stream back-pressure
/// keeps consumption ordered; the loop also naturally stops when the producer
/// signals completion. This example intentionally treats a stream error as
/// termination because it is illustrating response shape, not error recovery.
async fn run() {
    tracing_subscriber::fmt::init();

    let Response::Stream(mut stream) = stream_response() else {
        return;
    };
    while let Some(Ok(frame)) = stream.next().await {
        info!(?frame, "received frame");
    }
}

fn main() -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run());
    Ok(())
}
