//! RESP frame type definition.

/// A RESP protocol frame.
///
/// Supports simple strings, integers, bulk strings (nullable), and arrays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RespFrame {
    /// Simple string prefixed with `+`.
    SimpleString(String),
    /// Integer prefixed with `:`.
    Integer(i64),
    /// Bulk string prefixed with `$`, nullable via `$-1`.
    BulkString(Option<Vec<u8>>),
    /// Array prefixed with `*`.
    Array(Vec<RespFrame>),
}
