//! Policy for the correlation a protocol-supplied stream terminator carries.
//!
//! The two connection paths hold terminators in different shapes, so the
//! difference in behaviour is named here instead of being implicit in each
//! dispatch site. A directly driven actor queues [`Envelope`] frames, which
//! carry a correlation identifier, and its terminator inherits the response's
//! request correlation before `before_send` runs. A prepared connection's
//! terminator is already a complete codec frame: it has no correlation
//! metadata, so the pipeline writes it unchanged and never restamps it.
//!
//! [`Envelope`]: crate::app::Envelope

use crate::correlation::CorrelatableFrame;

/// What a response terminator's correlation identifier inherits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminatorCorrelation {
    /// Stamp the response's request correlation onto an envelope terminator.
    StampRequest,
    /// Write a complete codec frame exactly as the protocol supplied it.
    PreserveFrame,
}

impl TerminatorCorrelation {
    /// The policy a directly driven actor applies to its terminator envelope.
    pub(crate) const ACTOR: Self = Self::StampRequest;

    /// The policy a prepared connection applies to its codec-frame terminator.
    pub(crate) const PREPARED: Self = Self::PreserveFrame;

    /// Apply the policy to an envelope terminator.
    ///
    /// Only [`Self::StampRequest`] changes the frame. The prepared path never
    /// holds an envelope terminator, so its policy leaves the frame untouched.
    pub(crate) fn apply<F: CorrelatableFrame>(self, frame: &mut F, correlation: Option<u64>) {
        if self == Self::PREPARED {
            return;
        }
        if let Some(identifier) = correlation {
            frame.set_correlation_id(Some(identifier));
        }
    }

    /// Apply the policy to a complete codec frame.
    ///
    /// A codec frame carries no correlation metadata, so the only valid policy
    /// on that path is [`Self::PreserveFrame`]. The assertion makes a caller
    /// that reaches a codec terminator with the envelope policy a programming
    /// error rather than a silent restamp attempt.
    pub(crate) fn apply_codec_frame<T>(self, _frame: &mut T) {
        debug_assert_eq!(
            self,
            Self::PREPARED,
            "a codec-frame terminator cannot be restamped"
        );
    }
}

#[cfg(test)]
mod tests {
    //! The correlation each terminator policy applies.

    use super::TerminatorCorrelation;
    use crate::app::Envelope;

    #[test]
    fn stamp_request_applies_the_response_correlation() {
        let mut terminator = Envelope::new(9, None, Vec::new());
        TerminatorCorrelation::ACTOR.apply(&mut terminator, Some(7));
        assert_eq!(terminator.correlation_id, Some(7));
    }

    #[test]
    fn stamp_request_keeps_the_frame_correlation_when_the_request_has_none() {
        let mut terminator = Envelope::new(9, Some(3), Vec::new());
        TerminatorCorrelation::ACTOR.apply(&mut terminator, None);
        assert_eq!(terminator.correlation_id, Some(3));
    }

    #[test]
    fn preserve_frame_leaves_an_envelope_terminator_unchanged() {
        let mut terminator = Envelope::new(9, None, Vec::new());
        TerminatorCorrelation::PREPARED.apply(&mut terminator, Some(7));
        assert_eq!(terminator.correlation_id, None);
    }

    #[test]
    fn preserve_frame_accepts_a_codec_frame() {
        let mut frame = vec![1_u8, 2, 3];
        TerminatorCorrelation::PREPARED.apply_codec_frame(&mut frame);
        assert_eq!(frame, vec![1, 2, 3]);
    }
}
