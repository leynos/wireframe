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

use std::{future::Future, io, sync::Arc};

use tokio::sync::Notify;

use crate::metrics::{Direction, inc_frames};

/// Renewal signal raised by a sink after each completed physical write.
///
/// The actor hands a sink one future per composed frame, but a transport may
/// turn that frame into several physical writes. Without a progress signal the
/// actor's idle grace would bound the whole frame, so a peer that accepts every
/// fragment slowly would still lose the response once the total transfer
/// outlived one grace. Reporting each accepted write lets the actor renew its
/// deadline, while a write that stops making progress still expires.
#[derive(Debug, Default)]
pub(crate) struct ProgressSignal {
    /// Coalescing wake-up; the actor re-arms its deadline per report.
    notify: Notify,
}

impl ProgressSignal {
    /// Report that the transport accepted one physical write.
    ///
    /// Notifications coalesce, which is sufficient here: the actor renews once
    /// per report it observes, and reports arriving faster than it can react
    /// collapse into the next renewal.
    pub(crate) fn report(&self) { self.notify.notify_one(); }

    /// Resolve once the sink reports progress.
    async fn progressed(&self) { self.notify.notified().await; }
}

/// Await the sink's next progress report.
///
/// A sink without a signal never makes progress observable, so the future stays
/// pending for the lifetime of the write: such a sink is bounded as a whole.
pub(super) async fn next_progress(signal: Option<&ProgressSignal>) {
    match signal {
        Some(reported) => reported.progressed().await,
        None => std::future::pending().await,
    }
}

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

    /// Signal raised after each physical write this sink accepts.
    ///
    /// The default reports nothing, so a sink that writes one frame per call is
    /// bounded as a whole. A transport sink that splits one composed frame into
    /// several physical writes returns its signal so that accepting a fragment
    /// renews the actor's idle grace.
    fn progress(&self) -> Option<Arc<ProgressSignal>> { None }
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
