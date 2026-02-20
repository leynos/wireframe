//! Core builder types for `WireframeApp`.

use std::{collections::HashMap, sync::Arc};

use tokio::sync::{OnceCell, mpsc};

use crate::{
    app::{
        builder_defaults::DEFAULT_READ_TIMEOUT_MS,
        envelope::{Envelope, Packet},
        error::Result,
        lifecycle::{ConnectionSetup, ConnectionTeardown},
        memory_budgets::MemoryBudgets,
        middleware_types::{Handler, Middleware},
    },
    app_data_store::AppDataStore,
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    hooks::WireframeProtocol,
    message_assembler::MessageAssembler,
    middleware::HandlerService,
    serializer::{BincodeSerializer, Serializer},
};

/// Configures routing and middleware for a `WireframeServer`.
///
/// The builder stores registered routes and middleware without enforcing an
/// ordering. Methods return [`Result<Self>`] so registrations can be chained
/// ergonomically.
pub struct WireframeApp<
    S: Serializer + Send + Sync = BincodeSerializer,
    C: Send + 'static = (),
    E: Packet = Envelope,
    F: FrameCodec = LengthDelimitedFrameCodec,
> {
    pub(in crate::app) handlers: HashMap<u32, Handler<E>>,
    pub(in crate::app) routes: OnceCell<Arc<HashMap<u32, HandlerService<E>>>>,
    pub(in crate::app) middleware: Vec<Box<dyn Middleware<E>>>,
    pub(in crate::app) serializer: S,
    pub(in crate::app) app_data: AppDataStore,
    pub(in crate::app) on_connect: Option<Arc<ConnectionSetup<C>>>,
    pub(in crate::app) on_disconnect: Option<Arc<ConnectionTeardown<C>>>,
    pub(in crate::app) protocol:
        Option<Arc<dyn WireframeProtocol<Frame = F::Frame, ProtocolError = ()>>>,
    pub(in crate::app) push_dlq: Option<mpsc::Sender<Vec<u8>>>,
    pub(in crate::app) codec: F,
    pub(in crate::app) read_timeout_ms: u64,
    pub(in crate::app) fragmentation: Option<crate::fragment::FragmentationConfig>,
    pub(in crate::app) message_assembler: Option<Arc<dyn MessageAssembler>>,
    pub(in crate::app) memory_budgets: Option<MemoryBudgets>,
}

impl<S, C, E, F> Default for WireframeApp<S, C, E, F>
where
    S: Serializer + Default + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec + Default,
{
    /// Initializes empty routes, middleware, and application data with the
    /// default serializer and no lifecycle hooks.
    fn default() -> Self {
        let codec = F::default();
        Self {
            handlers: HashMap::new(),
            routes: OnceCell::new(),
            middleware: Vec::new(),
            serializer: S::default(),
            app_data: AppDataStore::default(),
            on_connect: None,
            on_disconnect: None,
            protocol: None,
            push_dlq: None,
            codec,
            read_timeout_ms: DEFAULT_READ_TIMEOUT_MS,
            fragmentation: None,
            message_assembler: None,
            memory_budgets: None,
        }
    }
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Default + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec + Default,
{
    /// Construct a new empty application builder.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses [`Result`] for
    /// forward compatibility.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::app::WireframeApp;
    /// WireframeApp::<_, _, wireframe::app::Envelope>::new().expect("failed to initialize app");
    /// ```
    pub fn new() -> Result<Self> { Ok(Self::default()) }

    /// Construct a new application builder using the provided serializer.
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but uses [`Result`] for
    /// forward compatibility.
    pub fn with_serializer(serializer: S) -> Result<Self> {
        Ok(Self {
            serializer,
            ..Self::default()
        })
    }
}

/// Groups the type-changing parameters for [`WireframeApp::rebuild_with_params`].
///
/// Consolidates serializer, codec, protocol, fragmentation, message assembler,
/// and memory budgets into a single value to keep the rebuild signature
/// concise.
pub(super) struct RebuildParams<S2, F2: FrameCodec> {
    pub(super) serializer: S2,
    pub(super) codec: F2,
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = F2::Frame, ProtocolError = ()>>>,
    pub(super) fragmentation: Option<crate::fragment::FragmentationConfig>,
    pub(super) message_assembler: Option<Arc<dyn MessageAssembler>>,
    pub(super) memory_budgets: Option<MemoryBudgets>,
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Helper to rebuild the app when changing type parameters.
    ///
    /// The `WireframeApp` builder carries 14 fields that must be moved together
    /// when swapping serializer or codec types. Centralising the reconstruction
    /// here keeps the transitions consistent and avoids repeating the same
    /// field list across each type-changing method. For smaller builders with
    /// only a handful of fields and single-field updates, prefer the macro-based
    /// pattern used by `WireframeClientBuilder`.
    pub(super) fn rebuild_with_params<S2, F2>(
        self,
        params: RebuildParams<S2, F2>,
    ) -> WireframeApp<S2, C, E, F2>
    where
        S2: Serializer + Send + Sync,
        F2: FrameCodec,
    {
        WireframeApp {
            handlers: self.handlers,
            routes: OnceCell::new(),
            middleware: self.middleware,
            serializer: params.serializer,
            app_data: self.app_data,
            on_connect: self.on_connect,
            on_disconnect: self.on_disconnect,
            protocol: params.protocol,
            push_dlq: self.push_dlq,
            codec: params.codec,
            read_timeout_ms: self.read_timeout_ms,
            fragmentation: params.fragmentation,
            message_assembler: params.message_assembler,
            memory_budgets: params.memory_budgets,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io, num::NonZeroUsize};

    use rstest::{fixture, rstest};

    use super::WireframeApp;
    use crate::{
        app::{Envelope, MemoryBudgets},
        codec::LengthDelimitedFrameCodec,
        serializer::BincodeSerializer,
    };

    type TestApp = WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>;

    #[fixture]
    fn app_builder() -> Result<TestApp, Box<dyn Error>> {
        let app = TestApp::new()
            .map_err(|error| io::Error::other(format!("failed to construct test app: {error}")))?;
        if app.fragmentation.is_some() {
            return Err(io::Error::other("fixture expects default fragmentation disabled").into());
        }
        if app.memory_budgets.is_some() {
            return Err(io::Error::other("fixture expects default memory budgets disabled").into());
        }
        Ok(app)
    }

    #[fixture]
    fn memory_budgets() -> Result<MemoryBudgets, Box<dyn Error>> {
        let per_message = NonZeroUsize::new(1024).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "message budget must be non-zero",
            )
        })?;
        let per_connection = NonZeroUsize::new(4096).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "connection budget must be non-zero",
            )
        })?;
        let in_flight = NonZeroUsize::new(2048).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "in-flight budget must be non-zero",
            )
        })?;
        Ok(MemoryBudgets::new(per_message, per_connection, in_flight))
    }

    #[rstest]
    fn builder_defaults_fragmentation_to_disabled(
        app_builder: Result<TestApp, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?;
        if app.fragmentation.is_some() {
            return Err(io::Error::other("fragmentation should be disabled by default").into());
        }
        Ok(())
    }

    #[rstest]
    fn builder_defaults_memory_budgets_to_disabled(
        app_builder: Result<TestApp, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?;
        if app.memory_budgets.is_some() {
            return Err(io::Error::other("memory budgets should be disabled by default").into());
        }
        Ok(())
    }

    #[rstest]
    fn enable_fragmentation_requires_explicit_opt_in(
        app_builder: Result<TestApp, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?.enable_fragmentation();
        if app.fragmentation.is_none() {
            return Err(
                io::Error::other("enable_fragmentation should configure fragmentation").into(),
            );
        }
        Ok(())
    }

    #[rstest]
    fn with_codec_clears_fragmentation_to_require_reconfiguration(
        app_builder: Result<TestApp, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?
            .enable_fragmentation()
            .with_codec(LengthDelimitedFrameCodec::new(2048));
        if app.fragmentation.is_some() {
            return Err(
                io::Error::other("with_codec should clear fragmentation configuration").into(),
            );
        }
        Ok(())
    }

    #[rstest]
    fn buffer_capacity_clears_fragmentation_to_require_reconfiguration(
        app_builder: Result<TestApp, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?.enable_fragmentation().buffer_capacity(2048);
        if app.fragmentation.is_some() {
            return Err(io::Error::other(
                "buffer_capacity should clear fragmentation configuration",
            )
            .into());
        }
        Ok(())
    }

    #[rstest]
    fn memory_budgets_builder_stores_configuration(
        app_builder: Result<TestApp, Box<dyn Error>>,
        memory_budgets: Result<MemoryBudgets, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let memory_budgets = memory_budgets?;
        let app = app_builder?.memory_budgets(memory_budgets);
        if app.memory_budgets != Some(memory_budgets) {
            return Err(
                io::Error::other("memory_budgets should store the configured value").into(),
            );
        }
        Ok(())
    }

    #[rstest]
    fn with_codec_preserves_memory_budgets(
        app_builder: Result<TestApp, Box<dyn Error>>,
        memory_budgets: Result<MemoryBudgets, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let memory_budgets = memory_budgets?;
        let app = app_builder?
            .memory_budgets(memory_budgets)
            .with_codec(LengthDelimitedFrameCodec::new(4096));
        if app.memory_budgets != Some(memory_budgets) {
            return Err(
                io::Error::other("with_codec should preserve configured memory budgets").into(),
            );
        }
        Ok(())
    }

    #[rstest]
    fn serializer_preserves_memory_budgets(
        app_builder: Result<TestApp, Box<dyn Error>>,
        memory_budgets: Result<MemoryBudgets, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let memory_budgets = memory_budgets?;
        let app = app_builder?
            .memory_budgets(memory_budgets)
            .serializer(BincodeSerializer);
        if app.memory_budgets != Some(memory_budgets) {
            return Err(
                io::Error::other("serializer should preserve configured memory budgets").into(),
            );
        }
        Ok(())
    }
}
