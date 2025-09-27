//! Traits for working with correlation identifiers on frames.
//!
//! `CorrelatableFrame` abstracts over frame types that carry an optional
//! correlation identifier, allowing generic components such as the connection
//! actor to stamp or inspect identifiers without knowing the concrete frame
//! representation.

/// Access and mutate correlation identifiers on frames.
pub trait CorrelatableFrame {
    /// Return the correlation identifier associated with this frame, if any.
    fn correlation_id(&self) -> Option<u64>;

    /// Set or clear the correlation identifier.
    fn set_correlation_id(&mut self, correlation_id: Option<u64>);
}

impl CorrelatableFrame for u8 {
    fn correlation_id(&self) -> Option<u64> { None }

    fn set_correlation_id(&mut self, _correlation_id: Option<u64>) {}
}

impl CorrelatableFrame for Vec<u8> {
    fn correlation_id(&self) -> Option<u64> { None }

    fn set_correlation_id(&mut self, _correlation_id: Option<u64>) {}
}
