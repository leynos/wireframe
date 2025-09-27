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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::app::Envelope;

    #[rstest]
    #[case(None)]
    #[case(Some(27))]
    fn envelope_correlation_round_trip(#[case] initial: Option<u64>) {
        let mut frame = Envelope::new(7, initial, vec![1, 2, 3]);
        assert_eq!(frame.correlation_id(), initial);

        frame.set_correlation_id(Some(99));
        assert_eq!(frame.correlation_id(), Some(99));

        frame.set_correlation_id(None);
        assert_eq!(frame.correlation_id(), None);
    }

    #[rstest]
    #[case::byte(0u8)]
    #[case::buffer(Vec::<u8>::new())]
    fn noop_implementations_ignore_correlation<T>(#[case] mut frame: T)
    where
        T: CorrelatableFrame,
    {
        assert_eq!(frame.correlation_id(), None);
        frame.set_correlation_id(Some(42));
        assert_eq!(frame.correlation_id(), None);
        frame.set_correlation_id(None);
        assert_eq!(frame.correlation_id(), None);
    }
}
