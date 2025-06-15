use bincode::error::{DecodeError, EncodeError};
use bincode::{BorrowDecode, Encode, borrow_decode_from_slice, config, encode_to_vec};

/// Wrapper trait for application message types.
///
/// Any type deriving [`Encode`] and [`BorrowDecode`] automatically implements
/// this trait via a blanket implementation. The default methods provide
/// convenient helpers to serialize and deserialize using bincode's
/// standard configuration.
pub trait Message: Encode + for<'de> BorrowDecode<'de, ()> {
    /// Serialize the message into a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an [`EncodeError`] if serialization fails.
    fn to_bytes(&self) -> Result<Vec<u8>, EncodeError> {
        encode_to_vec(self, config::standard())
    }

    /// Deserialize a message from a byte slice, returning the message and
    /// the number of bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if deserialization fails.
    fn from_bytes(bytes: &[u8]) -> Result<(Self, usize), DecodeError>
    where
        Self: Sized,
    {
        borrow_decode_from_slice(bytes, config::standard())
    }
}

impl<T> Message for T where for<'de> T: Encode + BorrowDecode<'de, ()> {}
