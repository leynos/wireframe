//! Async frame sinks used by the connection actor.
//!
//! The actor applies the composition it owns — correlation stamping, its
//! configured fragmenter, and its protocol hooks — before handing a frame to a
//! [`FrameSink`]. A sink may instead own part of that composition when the
//! actor is built without a fragmenter or `before_send` hook, which is how the
//! server connection keeps fragmentation, metrics, serialization, codec
//! wrapping, and `before_send` in one fixed order. The sink is awaited, so a
//! sink writing to a bounded transport transmits back-pressure to the actor's
//! data sources instead of buffering without limit.

use std::{future::Future, io};

use crate::metrics::{Direction, inc_frames};

/// Destination for fully composed frames emitted by the connection actor.
///
/// Implementations receive a frame after the actor has applied its own
/// fragmenter and `before_send` hook, which are absent when the sink owns that
/// composition. [`send_frame`](Self::send_frame) is awaited before the actor
/// polls its sources again, which is what applies transport back-pressure to
/// producers.
pub(crate) trait FrameSink<F> {
    /// Write one frame, awaiting any capacity the transport requires.
    fn send_frame(&mut self, frame: F) -> impl Future<Output = io::Result<()>> + Send;
}

/// Buffered sink retaining emitted frames for inspection.
///
/// This backs the standalone [`ConnectionActor::run`] driver, so it counts
/// each physical frame as outbound. An integrated transport sink leaves that
/// accounting to the transport it wraps.
///
/// [`ConnectionActor::run`]: super::ConnectionActor::run
impl<F> FrameSink<F> for Vec<F>
where
    F: Send + 'static,
{
    fn send_frame(&mut self, frame: F) -> impl Future<Output = io::Result<()>> + Send {
        self.push(frame);
        inc_frames(Direction::Outbound);
        std::future::ready(Ok(()))
    }
}
