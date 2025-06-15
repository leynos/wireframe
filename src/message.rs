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
    /// Deserialises a message instance from a byte slice using the standard configuration.
    ///
    /// Returns the deserialised message and the number of bytes consumed, or a [`DecodeError`] if deserialisation fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use your_crate::Message;
    /// let bytes = /* some valid serialised message bytes */;
    /// let (msg, consumed) = MyMessageType::from_bytes(&bytes).unwrap();
    /// assert!(consumed <= bytes.len());
    /// ```
    fn from_bytes(bytes: &[u8]) -> Result<(Self, usize), DecodeError>
    where
        Self: Sized,
    {
        borrow_decode_from_slice(bytes, config::standard())
    }
}

impl<T> Message for T where T: Encode + for<'de> BorrowDecode<'de, ()> {}
