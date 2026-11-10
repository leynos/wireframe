//! Connection-local runtime and at-most-once lifecycle finalization.
//!
//! A [`PreparedApp`](crate::app::PreparedApp) is immutable and shared by every
//! connection task. This module supplies the other half of that ownership
//! split: one [`ConnectionRuntime`] per accepted connection owns the mutable
//! stream, framed codec state, inbound pipeline, reassembly and assembly state,
//! the malformed-envelope counter, the resolved memory budgets, the read
//! timeout, and the setup state `C` produced by the connection lifecycle hook.
//! Nothing here is shared between connection tasks, and no field is wrapped in
//! `Arc<Mutex<_>>`.
//!
//! Finalization is a three-state machine rather than a `Drop` guard:
//!
//! - `NotSetUp` means no setup hook ran, so no teardown is owed;
//! - `Armed(C)` means setup produced state and teardown is owed exactly once;
//! - `Finalized` means the state has been taken and teardown must not run again.
//!
//! The driver takes the state and marks the finalizer `Finalized` *before*
//! invoking the asynchronous teardown callback. Consequently a teardown panic
//! cannot run teardown twice, and an error followed by outer task finalization
//! cannot repeat it either.
//!
//! # Ordering
//!
//! The runtime stages the raw transport and the immutable inbound
//! configuration, then builds [`InboundState`] when [`ConnectionRuntime::run`]
//! is driven. Setup therefore completes before the connection codec is cloned
//! or the maximum frame buffer is reserved: a setup hook that is still pending
//! cannot hold up to a frame's worth of buffer per connection, and a custom
//! codec is not initialized before setup establishes its state. Initialization
//! happens inside the panic-finalization boundary, so a panic during
//! initialization still releases the armed setup value.
//!
//! Inbound processing owns the framed stream. When processing stops, the
//! inbound state is dropped before teardown is awaited, which releases the
//! transport first. A teardown callback that waits for the peer to observe
//! closure can therefore make progress instead of deadlocking against a
//! transport this runtime still holds.
//!
//! # Cancellation
//!
//! Cooperative cancellation is observed only at the read boundary. A frame
//! that has already been read is decoded and dispatched to completion before
//! cancellation is noticed. Connection actors have their own cancellation
//! observations; this token is distinct from server accept-loop shutdown and
//! graceful drain.
//!
//! # Accepted gaps
//!
//! The guarantee covers every terminal outcome that resumes execution inside
//! the connection task: clean end of stream, decode and protocol errors,
//! transport read and write errors, read-timeout-driven continuation (which
//! does not terminate the connection), cooperative cancellation, and a handler
//! panic recovered by this runtime. Handlers cannot return errors; a handler
//! failure means the handler panicked.
//!
//! The following terminal paths cannot safely await an async teardown and are
//! accepted, documented gaps:
//!
//! - the connection future is dropped, or its Tokio task is aborted, while processing or teardown
//!   is pending;
//! - setup panics, or never completes and is dropped by its caller;
//! - teardown itself panics, in which case it is not retried; and
//! - builds compiled with `panic = "abort"`, where no unwinding or cleanup point exists at all.
//!
//! A read timeout purges expired outbound and assembly state and continues
//! waiting; it is not a terminal outcome.

mod decode;
mod finalizer;
mod inbound;

#[cfg(test)]
#[path = "finalizer_properties.rs"]
mod finalizer_properties;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ordering_tests.rs"]
mod ordering_tests;

use std::{io, panic::AssertUnwindSafe, sync::Arc};

// Only the inbound-handler tests decode envelopes directly; production code
// reaches the same function through `build_dispatchable_envelope`.
#[cfg(test)]
pub(crate) use decode::parse_envelope;
use finalizer::ConnectionFinalizer;
use futures::FutureExt;
pub(crate) use inbound::{InboundConfig, InboundState};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

use crate::{
    app::{
        envelope::{Envelope, Packet},
        lifecycle::ConnectionTeardown,
    },
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    serializer::Serializer,
};

/// Mutable state and at-most-once finalization for one accepted connection.
///
/// A connection task creates exactly one runtime, drives it to completion, and
/// drops it afterwards. The value is deliberately not `Clone`.
pub(crate) struct ConnectionRuntime<'a, S, C, E, F, W>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Transport owned until inbound state is built for processing.
    stream: W,
    /// Immutable template configuration borrowed for the connection lifetime.
    config: InboundConfig<'a, S, E, F>,
    /// Sole owner of the setup state until teardown takes it.
    finalizer: ConnectionFinalizer<C>,
}

impl<'a, S, C, E, F, W> ConnectionRuntime<'a, S, C, E, F, W>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Construct a runtime that owes no teardown until setup state is armed.
    ///
    /// Construction only stages the transport and configuration. The framed
    /// inbound state is built later by [`ConnectionRuntime::run`], so calling
    /// this before awaiting a setup hook cannot clone the codec or reserve the
    /// frame buffer.
    pub(crate) const fn new(stream: W, config: InboundConfig<'a, S, E, F>) -> Self {
        Self {
            stream,
            config,
            finalizer: ConnectionFinalizer::not_set_up(),
        }
    }

    /// Record the state produced by a successful setup hook.
    ///
    /// Only a fresh `NotSetUp` finalizer accepts state; callers arm exactly
    /// once, immediately after setup resolves.
    pub(crate) fn arm_setup(&mut self, state: C) { self.finalizer.arm(state); }

    /// Drive inbound processing, then finalize exactly once.
    ///
    /// Initialization happens here, inside `catch_unwind`, rather than during
    /// construction: setup has already completed, and a panic while building
    /// the inbound state still finalizes the armed setup value. `process`
    /// consumes the staged transport and drops the framed stream when it
    /// returns or unwinds, so the transport is released before teardown is
    /// awaited.
    ///
    /// Only the inbound processing future is wrapped in `catch_unwind`.
    /// `AssertUnwindSafe` is sound here because handlers never observe `C`,
    /// and the inbound state is dropped during unwinding without being reused
    /// after a panic. Setup and teardown panics are intentionally *not*
    /// caught: setup has not produced state to release, and teardown has
    /// already disarmed the finalizer before it runs.
    ///
    /// A processing panic is re-raised after finalization so the spawner's
    /// existing supervision still observes, counts, and logs it.
    pub(crate) async fn run(
        self,
        cancellation: Option<&CancellationToken>,
        teardown: Option<&Arc<ConnectionTeardown<C>>>,
    ) -> io::Result<()> {
        let Self {
            stream,
            config,
            mut finalizer,
        } = self;
        let processing_outcome = AssertUnwindSafe(Self::process(stream, config, cancellation))
            .catch_unwind()
            .await;

        finalizer.finalize(teardown).await;

        match processing_outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Build the connection-local inbound state and drive it to completion.
    ///
    /// The framed stream is a local value. Dropping it before returning, or
    /// during an unwind, releases the transport before the caller awaits
    /// teardown.
    async fn process(
        stream: W,
        config: InboundConfig<'a, S, E, F>,
        cancellation: Option<&CancellationToken>,
    ) -> io::Result<()> {
        let mut inbound = InboundState::new(stream, config);
        inbound.run(cancellation).await
    }
}
