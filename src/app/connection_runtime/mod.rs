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
    /// Everything the inbound reader owns or mutably borrows.
    inbound: InboundState<'a, S, E, F, W>,
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
    pub(crate) fn new(stream: W, config: InboundConfig<'a, S, E, F>) -> Self {
        Self {
            inbound: InboundState::new(stream, config),
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
    /// Only the inbound processing future is wrapped in `catch_unwind`.
    /// `AssertUnwindSafe` is sound here because handlers never observe `C`,
    /// and the runtime drops the inbound state without reusing it after a
    /// panic. Setup and teardown panics are intentionally *not* caught: setup
    /// has not produced state to release, and teardown has already disarmed
    /// the finalizer before it runs.
    ///
    /// A processing panic is re-raised after finalization so the spawner's
    /// existing supervision still observes, counts, and logs it.
    pub(crate) async fn run(
        &mut self,
        cancellation: Option<&CancellationToken>,
        teardown: Option<&Arc<ConnectionTeardown<C>>>,
    ) -> io::Result<()> {
        let processing_outcome = AssertUnwindSafe(self.inbound.run(cancellation))
            .catch_unwind()
            .await;

        self.finalizer.finalize(teardown).await;

        match processing_outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
