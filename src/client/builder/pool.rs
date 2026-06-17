//! Pooled connection establishment for `WireframeClientBuilder`.

use std::net::SocketAddr;

use bincode::Encode;

use super::WireframeClientBuilder;
use crate::{
    client::{
        ClientError,
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
        WireframeClientPool::connect(addr, pool_config, self.into_parts()).await
    }
}
