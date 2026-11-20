//! The reading one measurement window produces.
//!
//! Kept in its own module so the window machinery does not also own the value
//! it reports; callers that only name the type do not pull in the allocator
//! statics.

/// Allocation traffic observed inside one measurement window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AllocationTraffic {
    /// Allocation and reallocation operations the window observed.
    pub operations: usize,
    /// Bytes the window requested from the allocator.
    ///
    /// A growing reallocation contributes only the growth, matching what
    /// `stats_alloc` records. The value therefore measures memory asked for,
    /// not memory copied or freed.
    pub bytes: usize,
}
