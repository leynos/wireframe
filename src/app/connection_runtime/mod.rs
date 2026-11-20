//! Connection-local runtime and at-most-once lifecycle finalization.
//!
//! A [`PreparedApp`](crate::app::PreparedApp) is immutable and shared by every
//! connection task. This module supplies the other half of that ownership
//! split: one [`ConnectionRuntime`] per accepted connection owns the transport
//! and drives the shared frame-processing loop, whose framed codec state,
//! inbound pipeline, reassembly and assembly state, malformed-envelope
//! counter, resolved memory budgets, and read timeout are all locals of that
//! one connection task. The runtime additionally owns the setup state `C`
//! produced by the connection lifecycle hook. Nothing here is shared between
//! connection tasks, and no field is wrapped in `Arc<Mutex<_>>`.
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
//! configuration, then builds framing state when [`ConnectionRuntime::run`] is
//! driven. Setup therefore completes before the connection codec is cloned or
//! the maximum frame buffer is reserved: a setup hook that is still pending
//! cannot hold up to a frame's worth of buffer per connection, and a custom
//! codec is not initialized before setup establishes its state. Initialization
//! happens inside the panic-finalization boundary, so a panic during
//! initialization still releases the armed setup value.
//!
//! Inbound processing owns the framed stream. When processing stops, that
//! framing is dropped before teardown is awaited, which releases the transport
//! first. A teardown callback that waits for the peer to observe closure can
//! therefore make progress instead of deadlocking against a transport this
//! runtime still holds.
//!
//! # Cancellation
//!
//! Cooperative cancellation is observed only at the read boundary. A frame
//! that has already been read is decoded and dispatched to completion before
//! cancellation is noticed. The server supplies its per-connection shutdown
//! signal here so a peer that stops reading cannot pin a graceful drain;
//! connection actors observe the same token for in-flight responses.
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

mod finalizer;

#[cfg(test)]
#[path = "finalizer_properties.rs"]
mod finalizer_properties;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ordering_tests.rs"]
mod ordering_tests;

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod diagnostics_tests;

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod dispatch_tests;

use std::{collections::HashMap, io, net::SocketAddr, panic::AssertUnwindSafe, sync::Arc};

use finalizer::{ConnectionFinalizer, FinalizationOutcome};
use futures::FutureExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;
use tracing::error;

use super::{
    envelope::{Envelope, Packet},
    inbound_handler::core::{StreamProcessingContext, process_stream},
    lifecycle::ConnectionTeardown,
    memory_budgets::MemoryBudgets,
};
use crate::{
    codec::FrameCodec,
    frame::FrameMetadata,
    hooks::{ProtocolHooks, WireframeProtocol},
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    push::PushQueuesBuilder,
    serializer::Serializer,
    session::SessionRegistry,
};

/// Immutable configuration borrowed by one connection's runtime.
///
/// Every field is template-owned and shared across connections; none of it is
/// mutated while the connection runs.
pub(crate) struct InboundConfig<'a, S, E, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
{
    /// Immutable middleware chains used to dispatch decoded envelopes.
    pub(crate) routes: &'a HashMap<u32, HandlerService<E>>,
    /// Serializer borrowed from the prepared root for the connection lifetime.
    pub(crate) serializer: &'a S,
    /// Codec configuration shared by all frames on the connection.
    pub(crate) codec: &'a F,
    /// Optional assembly strategy for multi-frame protocol messages.
    pub(crate) message_assembler: Option<&'a dyn MessageAssembler>,
    /// Fragmentation settings used to initialize the frame pipeline.
    pub(crate) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Protocol hooks built for this connection from the installed protocol.
    pub(crate) protocol_hooks: ProtocolHooks<F::Frame, ()>,
    /// Installed protocol, used only to dispatch `on_connection_setup`.
    ///
    /// The runtime releases this reference once setup has run, so the
    /// connection's pipeline remains its single strong protocol owner.
    pub(crate) protocol: Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional byte budgets enforced while processing the connection.
    pub(crate) memory_budgets: Option<MemoryBudgets>,
    /// Maximum interval to wait for the next inbound frame.
    pub(crate) read_timeout_ms: u64,
    /// Per-connection shutdown signal; cancelling it stops this connection.
    pub(crate) shutdown: &'a CancellationToken,
    /// Template cloned to build this connection's push queues.
    pub(crate) push_queues: PushQueuesBuilder<F::Frame>,
    /// Registry this connection publishes its push handle to.
    pub(crate) session_registry: Arc<SessionRegistry<F::Frame>>,
    /// Peer address of the accepted socket, when the server reported one.
    pub(crate) peer_addr: Option<SocketAddr>,
}

/// Panic payload recovered by [`futures::FutureExt::catch_unwind`].
///
/// Naming it keeps the finalization helpers readable and makes it obvious that
/// the runtime forwards the payload rather than inspecting it.
type PanicPayload = Box<dyn std::any::Any + Send>;

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
    /// Transport owned until the processing loop is driven.
    stream: W,
    /// Immutable template configuration borrowed for the connection lifetime.
    config: InboundConfig<'a, S, E, F>,
    /// Sole owner of the setup state until teardown takes it.
    finalizer: ConnectionFinalizer<C>,
}

impl<'a, S, C, E, F, W> ConnectionRuntime<'a, S, C, E, F, W>
where
    S: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Send + Unpin,
    Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Construct a runtime that owes no teardown until setup state is armed.
    ///
    /// Construction only stages the transport and configuration. Framing is
    /// built later by [`ConnectionRuntime::run`], so calling this before
    /// awaiting a setup hook cannot clone the codec or reserve the frame
    /// buffer.
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
    /// framing state still finalizes the armed setup value. `process` consumes
    /// the staged transport and drops the framed stream when it returns or
    /// unwinds, so the transport is released before teardown is awaited.
    ///
    /// Only the inbound processing future is wrapped in `catch_unwind`.
    /// `AssertUnwindSafe` is sound here because handlers never observe `C`,
    /// and the framing state is dropped during unwinding without being reused
    /// after a panic. Setup and teardown panics are intentionally *not*
    /// caught: setup has not produced state to release, and teardown has
    /// already disarmed the finalizer before it runs.
    ///
    /// A processing panic is logged as a bounded diagnostic *before* teardown
    /// is awaited, then re-raised after finalization so the spawner's existing
    /// supervision still observes, counts, and logs it with its payload. The
    /// metric stays in supervision, so the panic is counted exactly once.
    ///
    /// The processing future is boxed so the runtime's own state machine holds
    /// a pointer rather than the whole frame-processing tree. Nesting that tree
    /// directly below the lifecycle wrapper makes the compiler's layout queries
    /// for a test crate exceed their depth limit, and it would otherwise be
    /// duplicated into every caller that joins two connections.
    pub(crate) async fn run(self, teardown: Option<&ConnectionTeardown<C>>) -> io::Result<()> {
        let Self {
            stream,
            config,
            mut finalizer,
        } = self;
        let processing_outcome = AssertUnwindSafe(Box::pin(Self::process(stream, config)))
            .catch_unwind()
            .await;
        let outcome = classify_processing_outcome(&processing_outcome);
        finalizer.finalize(teardown, outcome).await;
        resolve_processing_outcome(processing_outcome)
    }

    /// Drive the shared frame-processing loop to completion.
    ///
    /// Framing state, the response actor, and the frame pipeline are all locals
    /// of the processing future. Dropping them before returning, or during an
    /// unwind, releases the transport before the caller awaits teardown.
    async fn process(stream: W, config: InboundConfig<'a, S, E, F>) -> io::Result<()> {
        let InboundConfig {
            routes,
            serializer,
            codec,
            message_assembler,
            fragmentation,
            protocol_hooks,
            protocol,
            memory_budgets,
            read_timeout_ms,
            shutdown,
            push_queues,
            session_registry,
            peer_addr,
        } = config;
        process_stream(
            stream,
            StreamProcessingContext {
                routes,
                serializer,
                codec,
                message_assembler,
                fragmentation,
                protocol_hooks,
                protocol,
                memory_budgets,
                read_timeout_ms,
                shutdown,
                push_queues,
                session_registry,
                peer_addr,
            },
        )
        .await
    }
}

/// Classify a processing result and record a recovered handler panic.
///
/// The diagnostic is emitted here, before teardown is awaited, so it exists
/// even when a teardown callback never completes. It carries no panic payload:
/// supervision logs the payload after the re-raise.
fn classify_processing_outcome(
    processing: &Result<io::Result<()>, PanicPayload>,
) -> FinalizationOutcome {
    match processing {
        Ok(Ok(())) => FinalizationOutcome::Clean,
        Ok(Err(_)) => FinalizationOutcome::ReturnedError,
        Err(_) => {
            error!(
                stage = "connection_runtime",
                operation = "process_connection",
                "connection processing panicked; finalizing before re-raise"
            );
            FinalizationOutcome::ProcessingPanic
        }
    }
}

/// Return the processing result, re-raising a recovered panic.
///
/// Finalization has already run by the time this is called, so the payload is
/// re-raised purely for supervision's existing logging and metric path.
fn resolve_processing_outcome(processing: Result<io::Result<()>, PanicPayload>) -> io::Result<()> {
    match processing {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
