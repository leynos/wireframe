//! Helpers for draining `Response::MultiPacket` values in tests.
//!
//! These utilities collect all frames from a [`Response::MultiPacket`] into a
//! `Vec`, enabling concise assertions in tests and Cucumber steps.

use wireframe::Response;

/// Collect all frames from a [`Response::MultiPacket`].
///
/// # Examples
///
/// ```rust
/// use tokio::sync::mpsc;
/// use wireframe::Response;
/// use wireframe_testing::collect_multi_packet;
///
/// # async fn demo() {
/// let (tx, rx) = mpsc::channel(4);
/// tx.send(1u8).await.expect("send");
/// drop(tx);
/// let frames = collect_multi_packet(Response::MultiPacket(rx)).await;
/// assert_eq!(frames, vec![1]);
/// # }
/// ```
#[must_use]
pub async fn collect_multi_packet<F, E>(resp: Response<F, E>) -> Vec<F> {
    match resp {
        Response::MultiPacket(mut rx) => {
            let mut frames = Vec::new();
            while let Some(frame) = rx.recv().await {
                frames.push(frame);
            }
            frames
        }
        _ => panic!("expected Response::MultiPacket"),
    }
}
