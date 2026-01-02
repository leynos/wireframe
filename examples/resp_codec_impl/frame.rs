//! RESP frame type definition.

/// A RESP protocol frame.
///
/// Supports simple strings, errors, integers, bulk strings (nullable), and
/// arrays (nullable). Null values are represented as `None` variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RespFrame {
    /// Simple string prefixed with `+`.
    SimpleString(String),
    /// Error prefixed with `-`.
    Error(String),
    /// Integer prefixed with `:`.
    Integer(i64),
    /// Bulk string prefixed with `$`, nullable via `$-1`.
    BulkString(Option<Vec<u8>>),
    /// Array prefixed with `*`, nullable via `*-1`.
    Array(Option<Vec<RespFrame>>),
}
