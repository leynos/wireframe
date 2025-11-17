//! Zero-based fragment positioning within logical messages.
//!
//! Provides [`FragmentIndex`], a type-safe wrapper around `u32` that offers
//! overflow-safe increment operations for tracking fragment order.

use std::num::TryFromIntError;

use bincode::{Decode, Encode};
use derive_more::{Display, From};

/// Zero-based ordinal describing a fragment's position within its message.
///
/// # Examples
///
/// ```
/// use wireframe::fragment::FragmentIndex;
/// let index = FragmentIndex::new(3);
/// assert_eq!(index.get(), 3);
/// assert!(index.checked_increment().is_some());
/// ```
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Encode, Decode, Display, From,
)]
#[display("{_0}")]
pub struct FragmentIndex(u32);

impl FragmentIndex {
    /// Construct an index from a `u32` value.
    #[must_use]
    pub const fn new(value: u32) -> Self { Self(value) }

    /// Return the first valid fragment index.
    #[must_use]
    pub const fn zero() -> Self { Self(0) }

    /// Return the underlying numeric value.
    #[must_use]
    pub const fn get(self) -> u32 { self.0 }

    /// Increment the index, returning `None` on overflow.
    #[must_use]
    pub fn checked_increment(self) -> Option<Self> { self.0.checked_add(1).map(Self) }
}

impl TryFrom<usize> for FragmentIndex {
    type Error = TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> { u32::try_from(value).map(Self) }
}

impl From<FragmentIndex> for u32 {
    fn from(value: FragmentIndex) -> Self { value.0 }
}
