use bincode::{Decode, Encode};

use super::{FragmentIndex, MessageId};

/// Header describing a single fragment.
///
/// `FragmentHeader` is agnostic of the underlying frame or serializer. It
/// captures just enough information for codecs to stitch fragments back
/// together while remaining small enough to copy by value.
///
/// # Examples
///
/// ```
/// use wireframe::fragment::{FragmentHeader, FragmentIndex, MessageId};
/// let header = FragmentHeader::new(MessageId::new(7), FragmentIndex::zero(), false);
/// assert_eq!(header.message_id().get(), 7);
/// assert_eq!(header.fragment_index().get(), 0);
/// assert!(!header.is_last_fragment());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
pub struct FragmentHeader {
    message_id: MessageId,
    fragment_index: FragmentIndex,
    is_last_fragment: bool,
}

impl FragmentHeader {
    /// Create a new fragment header.
    #[must_use]
    pub const fn new(
        message_id: MessageId,
        fragment_index: FragmentIndex,
        is_last_fragment: bool,
    ) -> Self {
        Self {
            message_id,
            fragment_index,
            is_last_fragment,
        }
    }

    /// Return the logical message identifier.
    #[must_use]
    pub const fn message_id(&self) -> MessageId { self.message_id }

    /// Return the fragment position relative to the message.
    #[must_use]
    pub const fn fragment_index(&self) -> FragmentIndex { self.fragment_index }

    /// Report whether this is the final fragment.
    #[must_use]
    pub const fn is_last_fragment(&self) -> bool { self.is_last_fragment }
}
