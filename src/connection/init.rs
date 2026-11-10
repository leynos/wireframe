//! Construction inputs shared by the tracked and untracked actor constructors.
//!
//! Both entry points accept the same four values and differ only in whether
//! the actor registers itself with the active-connection gauge. Bundling the
//! inputs keeps that distinction visible instead of duplicating a long
//! parameter list.

use tokio_util::sync::CancellationToken;

use super::{ConnectionChannels, ProtocolHooks};
use crate::response::FrameStream;

/// Construction inputs shared by the tracked and untracked actor
/// constructors.
pub(super) struct ActorInit<F, E> {
    /// Push queues and the handle that keeps them open.
    pub(super) channels: ConnectionChannels<F>,
    /// Optional response stream installed at construction time.
    pub(super) response: Option<FrameStream<F, E>>,
    /// Cancellation token observed by the actor loop.
    pub(super) shutdown: CancellationToken,
    /// Protocol hooks applied to outbound frames.
    pub(super) hooks: ProtocolHooks<F, E>,
}

impl<F, E> ActorInit<F, E> {
    /// Bundle the construction inputs both constructors accept.
    pub(super) const fn new(
        channels: ConnectionChannels<F>,
        response: Option<FrameStream<F, E>>,
        shutdown: CancellationToken,
        hooks: ProtocolHooks<F, E>,
    ) -> Self {
        Self {
            channels,
            response,
            shutdown,
            hooks,
        }
    }
}
