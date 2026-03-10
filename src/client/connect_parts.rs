//! Shared connection construction parts for single and pooled clients.
//!
//! This module centralizes socket setup, preamble exchange, codec
//! configuration, and lifecycle hook execution so single-connection clients
//! and pooled connections share the exact same physical-connection semantics.

use std::{net::SocketAddr, sync::atomic::AtomicU64};

use bincode::Encode;
use tokio::net::{TcpSocket, TcpStream};
use tokio_util::codec::Framed;

use super::{
    ClientCodecConfig,
    ClientError,
    WireframeClient,
    hooks::{LifecycleHooks, RequestHooks},
    preamble_exchange::{PreambleConfig, perform_preamble_exchange},
    tracing_config::TracingConfig,
};
use crate::{rewind_stream::RewindStream, serializer::Serializer};

const INITIAL_READ_BUFFER_CAPACITY: usize = 64 * 1024;

/// Cloneable connection recipe used by pooled reconnect paths.
pub(crate) struct ClientBuildParts<S, P, C> {
    pub(crate) serializer: S,
    pub(crate) codec_config: ClientCodecConfig,
    pub(crate) socket_options: super::SocketOptions,
    pub(crate) preamble_config: Option<PreambleConfig<P>>,
    pub(crate) lifecycle_hooks: LifecycleHooks<C>,
    pub(crate) request_hooks: RequestHooks,
    pub(crate) tracing_config: TracingConfig,
}

impl<S, P, C> Clone for ClientBuildParts<S, P, C>
where
    S: Clone,
    P: Clone,
{
    fn clone(&self) -> Self {
        Self {
            serializer: self.serializer.clone(),
            codec_config: self.codec_config,
            socket_options: self.socket_options,
            preamble_config: self.preamble_config.clone(),
            lifecycle_hooks: self.lifecycle_hooks.clone(),
            request_hooks: self.request_hooks.clone(),
            tracing_config: self.tracing_config.clone(),
        }
    }
}

impl<S, P, C> ClientBuildParts<S, P, C>
where
    S: Serializer + Send + Sync,
    P: Encode + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Establish one physical client connection from the captured recipe.
    pub(crate) async fn connect(
        self,
        addr: SocketAddr,
    ) -> Result<WireframeClient<S, RewindStream<TcpStream>, C>, ClientError> {
        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        self.socket_options.apply(&socket)?;
        let mut stream = socket.connect(addr).await?;

        let leftover = if let Some(config) = self.preamble_config {
            perform_preamble_exchange(&mut stream, config).await?
        } else {
            Vec::new()
        };

        let codec_config = self.codec_config;
        let codec = codec_config.build_codec();
        let mut framed = Framed::new(RewindStream::new(leftover, stream), codec);
        let initial_read_buffer_capacity = core::cmp::min(
            INITIAL_READ_BUFFER_CAPACITY,
            codec_config.max_frame_length_value(),
        );
        framed
            .read_buffer_mut()
            .reserve(initial_read_buffer_capacity);

        let connection_state = if let Some(ref setup) = self.lifecycle_hooks.on_connect {
            Some(setup().await)
        } else {
            None
        };

        Ok(WireframeClient {
            framed,
            serializer: self.serializer,
            codec_config,
            connection_state,
            on_disconnect: self.lifecycle_hooks.on_disconnect,
            on_error: self.lifecycle_hooks.on_error,
            request_hooks: self.request_hooks,
            tracing_config: self.tracing_config,
            correlation_counter: AtomicU64::new(1),
        })
    }
}
