use bincode::{Decode, Encode};
use derive_more::{Display, From, Into};

/// Unique identifier for a logical message undergoing fragmentation.
///
/// # Examples
///
/// ```
/// use wireframe::fragment::MessageId;
/// let id = MessageId::new(42);
/// assert_eq!(id.get(), 42);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode, Display, From, Into)]
#[display("{_0}")]
pub struct MessageId(u64);

impl MessageId {
    /// Create a new identifier.
    #[must_use]
    pub const fn new(value: u64) -> Self { Self(value) }

    /// Return the inner numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}
