//! The completion sequence a prepared connection owes after a response.
//!
//! A handler response on the prepared path ends with three callbacks in one
//! fixed order: the protocol error, if the response failed; the protocol
//! terminator, if the response completed and was streaming; and the completion
//! hook, unless the response was cancelled. Naming that sequence here keeps the
//! order in one place instead of leaving it implicit in the call order of
//! `write_response`, and it keeps every callback on the connection's single
//! hook owner.

use crate::connection::ResponseOutcome;

/// The callbacks that follow a response body, in dispatch order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompletionSequence {
    /// Whether the protocol error hook runs first.
    reports_error: bool,
    /// Whether a stream terminator is written after the error hook.
    writes_terminator: bool,
    /// Whether the completion hook runs once the terminator has been written.
    completes: bool,
}

impl CompletionSequence {
    /// Derive the sequence a finished response owes.
    ///
    /// A failed response reports its protocol error; a failed stream is not
    /// terminated, so a client cannot read its truncation as the end of a
    /// complete stream. A completed streaming response is terminated. A
    /// cancelled response belongs to a closing connection and completes
    /// nothing.
    pub(crate) const fn for_outcome(outcome: ResponseOutcome, is_streaming: bool) -> Self {
        let failed = matches!(outcome, ResponseOutcome::Failed);
        let completed = matches!(outcome, ResponseOutcome::Completed);
        Self {
            reports_error: failed,
            writes_terminator: completed && is_streaming,
            completes: !matches!(outcome, ResponseOutcome::Cancelled),
        }
    }

    /// Whether the protocol error hook runs for this response.
    pub(crate) const fn reports_error(self) -> bool { self.reports_error }

    /// Whether a stream terminator is written for this response.
    pub(crate) const fn writes_terminator(self) -> bool { self.writes_terminator }

    /// Whether the completion hook runs for this response.
    pub(crate) const fn completes(self) -> bool { self.completes }
}

#[cfg(test)]
mod tests {
    //! The completion sequence each response outcome produces.

    use crate::{
        app::frame_handling::completion_sequence::CompletionSequence,
        connection::ResponseOutcome,
    };

    #[test]
    fn a_completed_stream_reports_no_error_terminates_and_completes() {
        let sequence = CompletionSequence::for_outcome(ResponseOutcome::Completed, true);
        assert!(!sequence.reports_error());
        assert!(sequence.writes_terminator());
        assert!(sequence.completes());
    }

    #[test]
    fn a_completed_single_response_completes_without_a_terminator() {
        let sequence = CompletionSequence::for_outcome(ResponseOutcome::Completed, false);
        assert!(!sequence.reports_error());
        assert!(!sequence.writes_terminator());
        assert!(sequence.completes());
    }

    #[test]
    fn a_failed_stream_reports_the_error_and_skips_the_terminator() {
        let sequence = CompletionSequence::for_outcome(ResponseOutcome::Failed, true);
        assert!(sequence.reports_error());
        assert!(!sequence.writes_terminator());
        assert!(sequence.completes());
    }

    #[test]
    fn a_cancelled_response_completes_nothing() {
        let sequence = CompletionSequence::for_outcome(ResponseOutcome::Cancelled, true);
        assert!(!sequence.reports_error());
        assert!(!sequence.writes_terminator());
        assert!(!sequence.completes());
    }
}
