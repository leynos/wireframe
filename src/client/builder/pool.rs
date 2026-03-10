//! Pooled connection establishment for `WireframeClientBuilder`.

use std::net::SocketAddr;

use bincode::Encode;

use super::WireframeClientBuilder;
use crate::{
    client::{
        ClientError,
        connect_parts::ClientBuildParts,
        pool::{ClientPoolConfig, WireframeClientPool},
    },
    serializer::Serializer,
};

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Establish a pooled client with warm reusable sockets.
    ///
    /// Each slot in the returned pool preserves preamble state until `bb8`
    /// recycles the idle socket.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the initial pool sockets cannot be created.
    pub async fn connect_pool(
        self,
        addr: SocketAddr,
        pool_config: ClientPoolConfig,
    ) -> Result<WireframeClientPool<S, P, C>, ClientError> {
        WireframeClientPool::connect(
            addr,
            pool_config,
            ClientBuildParts {
                serializer: self.serializer,
                codec_config: self.codec_config,
                socket_options: self.socket_options,
                preamble_config: self.preamble_config,
                lifecycle_hooks: self.lifecycle_hooks,
                request_hooks: self.request_hooks,
                tracing_config: self.tracing_config,
            },
        )
        .await
    }
}
