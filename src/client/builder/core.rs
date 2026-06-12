//! Core wireframe client builder type.

use crate::{
    client::{
        ClientCodecConfig,
        SocketOptions,
        connect_parts::ClientBuildParts,
        hooks::{LifecycleHooks, RequestHooks},
        preamble_exchange::PreambleConfig,
        tracing_config::TracingConfig,
    },
    serializer::BincodeSerializer,
};

/// Builder for [`WireframeClient`](crate::client::WireframeClient).
///
/// The builder supports three generic type parameters:
/// - `S`: The serializer type (default: `BincodeSerializer`)
/// - `P`: The preamble type (default: `()`)
/// - `C`: The connection state type returned by the setup hook (default: `()`)
///
/// # Examples
///
/// ```
/// use wireframe::client::WireframeClientBuilder;
///
/// let builder = WireframeClientBuilder::new();
/// let _ = builder;
/// ```
pub struct WireframeClientBuilder<S = BincodeSerializer, P = (), C = ()> {
    pub(crate) serializer: S,
    pub(crate) codec_config: ClientCodecConfig,
    pub(crate) socket_options: SocketOptions,
    pub(crate) preamble_config: Option<PreambleConfig<P>>,
    pub(crate) lifecycle_hooks: LifecycleHooks<C>,
    pub(crate) request_hooks: RequestHooks,
    pub(crate) tracing_config: TracingConfig,
}

impl WireframeClientBuilder<BincodeSerializer, (), ()> {
    /// Create a new builder with default settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new();
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            serializer: BincodeSerializer,
            codec_config: ClientCodecConfig::default(),
            socket_options: SocketOptions::default(),
            preamble_config: None,
            lifecycle_hooks: LifecycleHooks::default(),
            request_hooks: RequestHooks::default(),
            tracing_config: TracingConfig::default(),
        }
    }
}

impl Default for WireframeClientBuilder<BincodeSerializer, (), ()> {
    fn default() -> Self { Self::new() }
}

impl<S, P, C> WireframeClientBuilder<S, P, C> {
    /// Consume the builder into the shared physical-connection recipe.
    pub(crate) fn into_parts(self) -> ClientBuildParts<S, P, C> {
        ClientBuildParts {
            serializer: self.serializer,
            codec_config: self.codec_config,
            socket_options: self.socket_options,
            preamble_config: self.preamble_config,
            lifecycle_hooks: self.lifecycle_hooks,
            request_hooks: self.request_hooks,
            tracing_config: self.tracing_config,
        }
    }
}
