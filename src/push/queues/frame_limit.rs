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
