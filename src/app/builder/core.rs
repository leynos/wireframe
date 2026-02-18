//! Core builder types for `WireframeApp`.

use std::{collections::HashMap, sync::Arc};

use tokio::sync::{OnceCell, mpsc};

use crate::{
    app::{
        builder_defaults::DEFAULT_READ_TIMEOUT_MS,
        envelope::{Envelope, Packet},
        error::Result,
        lifecycle::{ConnectionSetup, ConnectionTeardown},
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
    ///
    /// let _app: WireframeApp = WireframeApp::new().expect("failed to initialize app");
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
/// Consolidates serializer, codec, protocol, fragmentation, and message
/// assembler into a single value to keep the rebuild signature concise.
pub(super) struct RebuildParams<S2, F2: FrameCodec> {
    pub(super) serializer: S2,
    pub(super) codec: F2,
    pub(super) protocol: Option<Arc<dyn WireframeProtocol<Frame = F2::Frame, ProtocolError = ()>>>,
    pub(super) fragmentation: Option<crate::fragment::FragmentationConfig>,
    pub(super) message_assembler: Option<Arc<dyn MessageAssembler>>,
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
    /// The `WireframeApp` builder carries 13 fields that must be moved together
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
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::WireframeApp;
    use crate::{app::Envelope, codec::LengthDelimitedFrameCodec, serializer::BincodeSerializer};

    type TestApp = WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>;

    #[fixture]
    fn app_builder() -> TestApp {
        let app = TestApp::new().expect("build app");
        assert!(
            app.fragmentation.is_none(),
            "fixture expects default fragmentation disabled"
        );
        app
    }

    #[rstest]
    fn builder_defaults_fragmentation_to_disabled(app_builder: TestApp) {
        let app = app_builder;
        assert!(app.fragmentation.is_none());
    }

    #[rstest]
    fn enable_fragmentation_requires_explicit_opt_in(app_builder: TestApp) {
        let app = app_builder.enable_fragmentation();
        assert!(app.fragmentation.is_some());
    }

    #[rstest]
    fn with_codec_clears_fragmentation_to_require_reconfiguration(app_builder: TestApp) {
        let app = app_builder
            .enable_fragmentation()
            .with_codec(LengthDelimitedFrameCodec::new(2048));
        assert!(app.fragmentation.is_none());
    }

    #[rstest]
    fn buffer_capacity_clears_fragmentation_to_require_reconfiguration(app_builder: TestApp) {
        let app = app_builder.enable_fragmentation().buffer_capacity(2048);
        assert!(app.fragmentation.is_none());
    }
}
