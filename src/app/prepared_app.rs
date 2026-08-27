//! Immutable application data prepared for connection handling.

use std::{collections::HashMap, sync::Arc};

use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::mpsc,
};

use super::{
    PrepareError,
    builder::WireframeApp,
    envelope::Packet,
    inbound_handler::{ConnectionProcessingContext, build_route_chains, process_connection},
    lifecycle::{ConnectionSetup, ConnectionTeardown},
    memory_budgets::MemoryBudgets,
};
use crate::{
    app_data_store::AppDataStore,
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    frame::FrameMetadata,
    hooks::WireframeProtocol,
    message::{DecodeWith, EncodeWith},
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

/// An immutable application template with fully transformed route services.
///
/// Obtain this type by consuming a [`WireframeApp`] with
/// [`WireframeApp::prepare`]. It deliberately has no route-registration API,
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
    pub(in crate::app) serializer: S,
    /// Codec template used to configure each connection's framed transport.
    pub(in crate::app) codec: F,
    #[expect(
        dead_code,
        reason = "connection-local request extraction will consume application data in the next \
                  runtime slice"
    )]
    /// Type-erased application state retained for the connection-runtime slice.
    pub(in crate::app) app_data: AppDataStore,
    /// Optional hook that creates per-connection state.
    pub(in crate::app) on_connect: Option<Arc<ConnectionSetup<C>>>,
    /// Optional hook that releases per-connection state after processing.
    pub(in crate::app) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    /// Optional protocol hook used to customize frame-level processing.
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    /// Optional assembler for protocol messages spread across several frames.
    pub(in crate::app) message_assembler: Option<Arc<dyn MessageAssembler>>,
    #[expect(
        dead_code,
        reason = "connection runtime ownership will consume the push dead-letter queue in a \
                  follow-up slice"
    )]
    /// Optional dead-letter sink for pushes that cannot be delivered.
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    /// Optional limits and timeout for transparent frame fragmentation.
    pub(in crate::app) fragmentation: Option<crate::fragment::FragmentationConfig>,
    /// Maximum interval to wait for the next inbound frame.
    pub(in crate::app) read_timeout_ms: u64,
    /// Optional byte caps protecting message and connection memory usage.
    pub(in crate::app) memory_budgets: Option<MemoryBudgets>,
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Consume builder registrations and prepare immutable route services.
    ///
    /// Middleware transforms run once for every registered route during this
    /// transition. The returned template can then drive multiple connections
    /// without rebuilding its route chains.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// # let _ = prepared;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`PrepareError`] if a future fallible preparation step fails.
    pub async fn prepare(self) -> Result<PreparedApp<S, C, E, F>, PrepareError> {
        let routes = build_route_chains(&self.handlers, &self.middleware).await;

        Ok(PreparedApp {
            routes,
            serializer: self.serializer,
            codec: self.codec,
            app_data: self.app_data,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            protocol: self.protocol,
            message_assembler: self.message_assembler,
            push_dlq: self.push_dlq,
            fragmentation: self.fragmentation,
            read_timeout_ms: self.read_timeout_ms,
            memory_budgets: self.memory_budgets,
        })
    }
}

impl<S, C, E, F> PreparedApp<S, C, E, F>
where
    S: Serializer + FrameMetadata<Frame = super::Envelope> + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
    super::Envelope: DecodeWith<S> + EncodeWith<S>,
{
    /// Handle an accepted connection using the prepared route services.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// prepared.handle_connection_result(server).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if stream processing or handler execution fails.
    pub async fn handle_connection_result<W>(&self, stream: W) -> io::Result<()>
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        process_connection(
            stream,
            ConnectionProcessingContext {
                routes: &self.routes,
                serializer: &self.serializer,
                codec: &self.codec,
                on_connect: self.on_connect.as_ref(),
                on_disconnect: self.on_disconnect.as_ref(),
                message_assembler: self.message_assembler.as_ref(),
                fragmentation: self.fragmentation,
                memory_budgets: self.memory_budgets,
                read_timeout_ms: self.read_timeout_ms,
            },
        )
        .await
    }

    /// Handle an accepted connection and log any processing failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tokio::io::duplex;
    /// use wireframe::app::{PreparedApp, WireframeApp};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    /// let (client, server) = duplex(64);
    /// drop(client);
    /// prepared.handle_connection(server).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle_connection<W>(&self, stream: W)
    where
        W: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        if let Err(error) = self.handle_connection_result(stream).await {
            log::warn!(
                "connection handling completed with error: correlation_id={:?}, error={error:?}",
                None::<u64>
            );
        }
    }
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
        self.protocol
            .as_ref()
            .map(crate::hooks::ProtocolHooks::from_protocol)
            .unwrap_or_default()
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
