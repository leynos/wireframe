//! Shared echo-login contract used by examples and client runtime tests.

/// Route identifier used by the echo-login example flow.
pub const LOGIN_ROUTE_ID: u32 = 1;

/// Client login payload sent to the echo server.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::BorrowDecode)]
pub struct LoginRequest {
    pub username: String,
}

/// Acknowledgement payload decoded from the echoed login message.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::BorrowDecode)]
pub struct LoginAck {
    pub username: String,
}
