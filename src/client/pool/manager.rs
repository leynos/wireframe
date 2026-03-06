//! `bb8` manager for one physical wireframe client connection.

use std::{future::Future, net::SocketAddr};

use bb8::ManageConnection;
use bincode::Encode;

use super::managed::ManagedClientConnection;
use crate::{
    client::{ClientError, connect_parts::ClientBuildParts},
    serializer::Serializer,
};

/// Connection manager for one pooled wireframe socket.
#[derive(Clone)]
pub(crate) struct WireframeConnectionManager<S, P, C> {
    addr: SocketAddr,
    parts: ClientBuildParts<S, P, C>,
}

impl<S, P, C> WireframeConnectionManager<S, P, C> {
    pub(crate) fn new(addr: SocketAddr, parts: ClientBuildParts<S, P, C>) -> Self {
        Self { addr, parts }
    }
}

impl<S, P, C> ManageConnection for WireframeConnectionManager<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    type Connection = ManagedClientConnection<S, C>;
    type Error = ClientError;

    fn connect(&self) -> impl Future<Output = Result<Self::Connection, Self::Error>> + Send {
        let parts = self.parts.clone();
        let addr = self.addr;
        async move { parts.connect(addr).await.map(ManagedClientConnection::new) }
    }

    fn is_valid(
        &self,
        conn: &mut Self::Connection,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let is_broken = conn.is_broken();
        async move {
            if is_broken {
                Err(ClientError::disconnected())
            } else {
                Ok(())
            }
        }
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool { conn.is_broken() }
}
