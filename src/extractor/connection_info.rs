//! Extractor for connection metadata.

use std::net::SocketAddr;

use super::{FromMessageRequest, MessageRequest, Payload};

/// Extractor providing peer connection metadata.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionInfo {
    peer_addr: Option<SocketAddr>,
}

impl ConnectionInfo {
    /// Returns the peer's socket address for the current connection, if available.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::extractor::{ConnectionInfo, FromMessageRequest, MessageRequest, Payload};
    ///
    /// let addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid socket address");
    /// let req = MessageRequest::new().with_peer_addr(Some(addr));
    /// let info = ConnectionInfo::from_message_request(&req, &mut Payload::default())
    ///     .expect("connection info extraction should succeed");
    /// assert_eq!(info.peer_addr(), Some(addr));
    /// ```
    #[must_use]
    pub fn peer_addr(&self) -> Option<SocketAddr> { self.peer_addr }
}

impl FromMessageRequest for ConnectionInfo {
    type Error = std::convert::Infallible;

    /// Extracts connection metadata from the message request.
    ///
    /// Returns a `ConnectionInfo` containing the peer's socket address, if available. This
    /// extraction is infallible.
    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            peer_addr: req.peer_addr,
        })
    }
}
