//! Shared echo-login contract used by examples and client runtime tests.

/// Route identifier used by the echo-login example flow.
pub const LOGIN_ROUTE_ID: u32 = 1;

/// Client login payload sent to the echo server.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::BorrowDecode)]
pub struct LoginRequest {
    /// User name carried in the request and echoed by the server as proof that
    /// the login handshake decoded the same payload on both sides.
    pub username: String,
}

/// Acknowledgement payload decoded from the echoed login message.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::BorrowDecode)]
pub struct LoginAck {
    /// User name returned by the server; the client compares it with its
    /// request before considering the handshake complete.
    pub username: String,
}
