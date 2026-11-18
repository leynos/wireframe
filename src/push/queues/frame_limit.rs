//! Payload-size budget applied to pushes before they are queued.
//!
//! A server connection writes queued pushes to the transport unchanged, so a
//! frame its codec cannot encode would fail after the producer had been told
//! the push succeeded, and that failure ends the connection. The budget is
//! checked at enqueue instead. Measurement is supplied by the codec as a
//! function because a frame type need not expose its own payload length.

use super::PushError;

/// Payload-size budget applied to a push before it is queued.
#[derive(Debug)]
pub(crate) struct FrameLimit<F> {
    /// Largest payload the connection's codec will encode.
    pub(crate) max: usize,
    /// Reports how many payload bytes one frame encodes to.
    pub(crate) payload_len: fn(&F) -> usize,
}

// Written by hand so a limit can be copied without requiring the frame type to
// be `Clone`; only the stored function pointer is copied.
impl<F> Clone for FrameLimit<F> {
    fn clone(&self) -> Self { *self }
}

impl<F> Copy for FrameLimit<F> {}

impl<F> FrameLimit<F> {
    /// Reject `frame` when its payload exceeds the budget.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::FrameTooLarge`] with the measured size.
    pub(crate) fn check(&self, frame: &F) -> Result<(), PushError> {
        let size = (self.payload_len)(frame);
        if size > self.max {
            return Err(PushError::FrameTooLarge {
                size,
                max: self.max,
            });
        }
        Ok(())
    }
}

/// Every payload-size budget one connection applies to a push.
///
/// A queue template and the connection's codec can measure a frame
/// differently, so each limit is kept with its own measure and both are
/// checked. Folding them into one numeric bound would apply one side's measure
/// to the other side's ceiling, which can accept a frame that either limit
/// would reject.
#[derive(Debug)]
pub(crate) struct FrameLimits<F> {
    /// Limit configured on the push-queue template, if any.
    template: Option<FrameLimit<F>>,
    /// Limit imposed by the codec that will encode the frames, if any.
    codec: Option<FrameLimit<F>>,
}

// Written by hand so an unbounded pair needs nothing from the frame type.
impl<F> Default for FrameLimits<F> {
    fn default() -> Self {
        Self {
            template: None,
            codec: None,
        }
    }
}

// Written by hand so a pair of limits copies without requiring the frame type
// to be `Clone`; only the stored function pointers are copied.
impl<F> Clone for FrameLimits<F> {
    fn clone(&self) -> Self { *self }
}

impl<F> Copy for FrameLimits<F> {}

impl<F> FrameLimits<F> {
    /// Store the template's limit, replacing any earlier template limit.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let limits = FrameLimits::default().with_template(limit);
    /// ```
    pub(crate) const fn with_template(mut self, limit: FrameLimit<F>) -> Self {
        self.template = Some(limit);
        self
    }

    /// Store the codec's limit alongside the template's.
    ///
    /// The codec limit is checked as well as, not instead of, the template
    /// limit, because the two may measure a frame differently.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let limits = FrameLimits::default().with_codec(codec_limit);
    /// ```
    pub(crate) const fn with_codec(mut self, limit: FrameLimit<F>) -> Self {
        self.codec = Some(limit);
        self
    }

    /// Reject `frame` when either configured limit rejects it.
    ///
    /// # Errors
    ///
    /// Returns [`PushError::FrameTooLarge`] from the first limit that rejects
    /// the frame, reporting that limit's own measure and ceiling.
    pub(crate) fn check(&self, frame: &F) -> Result<(), PushError> {
        if let Some(limit) = &self.template {
            limit.check(frame)?;
        }
        self.codec
            .as_ref()
            .map_or(Ok(()), |limit| limit.check(frame))
    }
}

#[cfg(test)]
mod tests {
    //! Unit and property coverage for the per-connection frame-size budget.

    use proptest::prelude::*;

    use super::{FrameLimit, FrameLimits};
    use crate::push::PushError;

    /// Measure a frame by the bytes it occupies.
    fn exact(frame: &usize) -> usize { *frame }

    /// Measure a frame at twice its bytes, to model a codec that adds framing.
    fn doubled(frame: &usize) -> usize { frame.saturating_mul(2) }

    /// A limit that measures frames exactly.
    fn limit(max: usize) -> FrameLimit<usize> {
        FrameLimit {
            max,
            payload_len: exact,
        }
    }

    /// The boundary the budget is named for: the limit itself is admissible.
    #[test]
    fn a_frame_at_the_limit_is_accepted() {
        let limits = FrameLimits::default().with_template(limit(4));

        assert_eq!(limits.check(&4), Ok(()));
    }

    /// One byte over the limit is rejected with the measured size and ceiling.
    #[test]
    fn a_frame_above_the_limit_reports_its_measurement() {
        let limits = FrameLimits::default().with_template(limit(4));

        assert_eq!(
            limits.check(&5),
            Err(PushError::FrameTooLarge { size: 5, max: 4 })
        );
    }

    // A template-only budget admits exactly the frames inside its bound.
    proptest! {
        #[test]
        fn a_template_limit_admits_exactly_frames_within_it(
            size in 0usize..2048,
            max in 0usize..2048,
        ) {
            let limits = FrameLimits::default().with_template(limit(max));

            prop_assert_eq!(limits.check(&size).is_ok(), size <= max);
        }
    }

    // A codec-only budget admits exactly the frames inside its bound.
    proptest! {
        #[test]
        fn a_codec_limit_admits_exactly_frames_within_it(
            size in 0usize..2048,
            max in 0usize..2048,
        ) {
            let limits = FrameLimits::default().with_codec(limit(max));

            prop_assert_eq!(limits.check(&size).is_ok(), size <= max);
        }
    }

    // A combined budget admits a frame only when both limits accept it, and the
    // rejection it reports belongs to the limit that rejected it.
    proptest! {
        #[test]
        fn combined_limits_admit_only_frames_both_accept(
            size in 0usize..2048,
            template_max in 0usize..2048,
            codec_max in 0usize..2048,
        ) {
            let limits = FrameLimits::default()
                .with_template(limit(template_max))
                .with_codec(limit(codec_max));
            let outcome = limits.check(&size);

            prop_assert_eq!(
                outcome.is_ok(),
                size <= template_max && size <= codec_max
            );
            if size > template_max {
                prop_assert_eq!(
                    outcome,
                    Err(PushError::FrameTooLarge {
                        size,
                        max: template_max,
                    })
                );
            } else if size > codec_max {
                prop_assert_eq!(
                    outcome,
                    Err(PushError::FrameTooLarge {
                        size,
                        max: codec_max,
                    })
                );
            }
        }
    }

    // Each side keeps its own measure, so a codec that counts framing overhead
    // can reject a frame the template's byte count would have admitted.
    proptest! {
        #[test]
        fn each_limit_applies_its_own_measure(
            size in 0usize..1024,
            template_max in 0usize..1024,
            codec_max in 0usize..2048,
        ) {
            let limits = FrameLimits::default()
                .with_template(limit(template_max))
                .with_codec(FrameLimit {
                    max: codec_max,
                    payload_len: doubled,
                });

            prop_assert_eq!(
                limits.check(&size).is_ok(),
                size <= template_max && size.saturating_mul(2) <= codec_max
            );
        }
    }
}
