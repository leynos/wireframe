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
