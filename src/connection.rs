//! Connection actor responsible for outbound frames.
//!
//! The actor polls a shutdown token, high- and low-priority push queues,
//! and an optional response stream using a `tokio::select!` loop. The
//! `biased` keyword ensures high-priority messages are processed before
//! low-priority ones, with streamed responses handled last.

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::{
    push::{FrameLike, PushQueues},
    response::{FrameStream, WireframeError},
};

/// Actor driving outbound frame delivery for a connection.
pub struct ConnectionActor<F, E> {
    pub queues: PushQueues<F>,
    pub response: Option<FrameStream<F, E>>, // current streaming response
    pub shutdown: CancellationToken,
}

impl<F, E> ConnectionActor<F, E>
where
    F: FrameLike,
{
    /// Create a new `ConnectionActor` from the provided components.
    #[must_use]
    pub fn new(
        queues: PushQueues<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            queues,
            response,
            shutdown,
        }
    }

    /// Drive the actor until all sources are exhausted or shutdown is triggered.
    ///
    /// Frames are appended to `out` in the order they are processed.
    ///
    /// # Errors
    ///
    /// Returns a [`WireframeError`] if the response stream yields an error.
    pub async fn run(&mut self, out: &mut Vec<F>) -> Result<(), WireframeError<E>> {
        let mut high_closed = false;
        let mut low_closed = false;
        let mut resp_closed = self.response.is_none();

        loop {
            tokio::select! {
                biased;

                () = self.shutdown.cancelled() => break,

                res = self.queues.high_priority_rx.recv(), if !high_closed => {
                    match res {
                        Some(frame) => out.push(frame),
                        None => high_closed = true,
                    }
                }

                res = self.queues.low_priority_rx.recv(), if !low_closed => {
                    match res {
                        Some(frame) => out.push(frame),
                        None => low_closed = true,
                    }
                }

                res = async {
                    if resp_closed {
                        None
                    } else if let Some(stream) = &mut self.response {
                        stream.next().await
                    } else {
                        None
                    }
                } => {
                    match res {
                        Some(Ok(frame)) => out.push(frame),
                        Some(Err(e)) => return Err(e),
                        None => resp_closed = true,
                    }
                }
            }

            if high_closed && low_closed && resp_closed {
                break;
            }
        }

        Ok(())
    }
}
