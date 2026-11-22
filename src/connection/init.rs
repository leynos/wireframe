//! Construction inputs shared by the tracked and untracked actor constructors.
//!
//! All entry points accept the same values and differ only in whether the actor
//! registers itself with the active-connection gauge, who runs the protocol
//! setup callback, and how queued push frames are composed. Bundling the inputs
//! keeps those distinctions visible instead of duplicating a long parameter
//! list.
//!
//! Hook ownership is part of those inputs. A caller that drives the actor
//! itself supplies [`ProtocolHooks`]; a prepared connection supplies `None`,
//! because its `FramePipeline` owns every callback. The actor's own dispatch
//! points are then compiled out rather than skipped at runtime.

use tokio_util::sync::CancellationToken;

use super::{ConnectionChannels, ProtocolHooks, pushes};
use crate::{hooks::ConnectionContext, response::FrameStream};

/// Construction inputs for an actor the caller constructs itself.
///
/// The push frame type is separate from the response frame type: a standalone
/// actor queues the same frame type it responds with, while the server queues
/// frames that are already encoded for the wire.
pub(super) struct ActorInit<F, E, P> {
    /// Push queues and the handle that keeps them open.
    pub(super) channels: ConnectionChannels<P>,
    /// Optional response stream installed at construction time.
    pub(super) response: Option<FrameStream<F, E>>,
    /// Cancellation token observed by the actor loop.
    pub(super) shutdown: CancellationToken,
    /// Protocol hooks applied to outbound frames, or `None` when another owner
    /// dispatches them for this connection.
    pub(super) hooks: Option<ProtocolHooks<F, E>>,
    /// Connection-local state handed to protocol callbacks.
    ///
    /// Ignored when `hooks` is `None`: an actor with no hook owner has no
    /// callback that could observe a context.
    pub(super) ctx: ConnectionContext,
    /// Composition applied to frames taken from the push queues.
    pub(super) composer: pushes::PushComposer<F, E, P>,
}

/// Construction inputs for an untracked actor a test builds directly.
///
/// The production server builds its actors through the prepared constructor,
/// which installs the no-hook mode; this input exists for tests that supply
/// their own hooks and context.
#[cfg(test)]
pub(crate) struct UntrackedActorInit<F, E, P> {
    /// Push queues and the handle that keeps them open.
    pub(crate) channels: ConnectionChannels<P>,
    /// Optional response stream installed at construction time.
    pub(crate) response: Option<FrameStream<F, E>>,
    /// Cancellation token observed by the actor loop.
    pub(crate) shutdown: CancellationToken,
    /// Protocol hooks applied to outbound frames.
    pub(crate) hooks: ProtocolHooks<F, E>,
    /// Connection-local state handed to protocol callbacks.
    pub(crate) ctx: ConnectionContext,
}
