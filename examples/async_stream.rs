//! Demonstrates generating `Response::Stream` values using `async-stream`.
//!
//! The `stream_response` function yields five sequential frames using
//! `async_stream::try_stream`. It returns a `Response` that can be
//! consumed by a `ConnectionActor`.

use async_stream::try_stream;
use futures::StreamExt;
use tracing::info;
use wireframe::response::Response;

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
struct Frame(u32);

fn stream_response() -> Response<Frame> {
    let frames = try_stream! {
        for n in 0..5u32 {
            yield Frame(n);
        }
    };
    Response::Stream(Box::pin(frames))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let Response::Stream(mut stream) = stream_response() else {
        return;
    };
    while let Some(Ok(frame)) = stream.next().await {
        info!(?frame, "received frame");
    }
}
