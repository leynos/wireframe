//! Immutable application data prepared for connection handling.

use std::{collections::HashMap, sync::Arc};

use tokio::sync::mpsc;

use super::{
    envelope::Packet,
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
};
use crate::{
    codec::{FrameCodec, LengthDelimitedFrameCodec, RecoveryConfig, RecoveryPolicyHook},
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

mod connection;
mod preparation;

/// An immutable application template with fully transformed route services.
///
/// Obtain this type by consuming a
/// [`WireframeApp`](crate::app::builder::WireframeApp) with
/// [`WireframeApp::prepare`](crate::app::builder::WireframeApp::prepare). It
/// deliberately has no route-registration API,
/// so all handler and middleware transforms have completed before connections
/// start using the application.
pub struct PreparedApp<
    S: Serializer + Send + Sync = BincodeSerializer,
    C: Send + 'static = (),
    E: Packet = super::Envelope,
    F: FrameCodec = LengthDelimitedFrameCodec,
> {
    /// Fully transformed route services keyed by protocol message identifier.
    pub(in crate::app) routes: HashMap<u32, HandlerService<E>>,
    /// Serializer retained by every connection driven by this application.
    ///
    /// Shared so the connection actor's `'static` protocol hooks can decode
    /// terminator frames without copying the serializer.
    pub(in crate::app) serializer: Arc<S>,
    /// Codec template used to configure each connection's framed transport.
    pub(in crate::app) codec: F,
    /// Optional hook that creates per-connection state.
    pub(in crate::app) on_connect: Option<Arc<ConnectionSetup<C>>>,
    /// Optional hook that releases per-connection state after processing.
    pub(in crate::app) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    /// Optional protocol hook used to customize frame-level processing.
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional assembler for protocol messages spread across several frames.
    pub(in crate::app) message_assembler: Option<Arc<dyn MessageAssembler>>,
    // FIXME: Issue #643 keeps this template-owned configuration until the
    // connection-local runtime consumes it: https://github.com/leynos/wireframe/issues/643.
    #[expect(
        dead_code,
        reason = "FIXME: #643 tracks ConnectionRuntime consuming the push DLQ"
    )]
    /// Optional dead-letter sink for pushes that cannot be delivered.
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    /// Optional limits and timeout for transparent frame fragmentation.
    pub(in crate::app) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Maximum interval to wait for the next inbound frame.
    pub(in crate::app) read_timeout_ms: u64,
    /// Optional byte caps protecting message and connection memory usage.
    pub(in crate::app) memory_budgets: Option<MemoryBudgets>,
    /// Optional hook consulted when a decoder fails on a live connection.
    pub(in crate::app) recovery_hook: Option<Arc<dyn RecoveryPolicyHook>>,
    /// Global recovery settings copied into each connection.
    pub(in crate::app) recovery_config: RecoveryConfig,
}

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Get a clone of the configured protocol, if any.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// assert!(prepared.protocol().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn protocol(
        &self,
    ) -> Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>> {
        self.protocol.clone()
    }

    /// Return protocol hooks derived from the installed protocol.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let hooks = prepared.protocol_hooks();
    /// # let _ = hooks;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn protocol_hooks(&self) -> crate::hooks::ProtocolHooks<F::Frame, ()> {
        self.protocol.as_ref().map_or_else(
            crate::hooks::ProtocolHooks::default,
            crate::hooks::ProtocolHooks::from_protocol,
        )
    }

    /// Get the configured message assembler, if any.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// assert!(prepared.message_assembler().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn message_assembler(&self) -> Option<&Arc<dyn MessageAssembler>> {
        self.message_assembler.as_ref()
    }
}

#[cfg(test)]
#[path = "../prepared_app_tests.rs"]
mod tests;
