//! Response container shared by handlers and middleware.
//!
//! [`ServiceResponse`] carries the frame bytes middleware may inspect, and,
//! for a route registered with
//! [`WireframeApp::route_response`](crate::app::WireframeApp::route_response), a
//! deferred [`Response`] body that owns a stream or channel. Keeping the
//! deferred machinery here leaves the middleware chain in the parent module.

use std::sync::{Arc, Mutex, PoisonError};

use super::FrameContainer;
use crate::{app::Envelope, response::Response};

/// Shared take-once slot holding a route's explicit response body.
///
/// Clones of a [`ServiceResponse`] share the slot, so whichever copy is
/// consumed first takes the body exactly once.
pub(super) type DeferredBody = Arc<Mutex<Option<Response<Envelope, ()>>>>;

/// Response produced by a handler or middleware.
///
/// A route registered with
/// [`WireframeApp::route_response`](crate::app::WireframeApp::route_response)
/// stores its [`Response`] body out of band. [`frame`](Self::frame) then stays
/// empty, and mutating it does not change what the connection actor writes.
/// Cloning shares that take-once body slot, so the first consumer takes the
/// body and later clones see none; equality and `Debug` ignore the body's
/// contents.
#[derive(Default)]
pub struct ServiceResponse {
    /// Payload returned by a handler and later encoded for transport.
    inner: FrameContainer<Vec<u8>>,
    /// Correlation metadata carried by the response.
    correlation_id: Option<u64>,
    /// Optional response body produced by a streaming route handler.
    ///
    /// The body is stored out of band because it may own a channel or stream
    /// that cannot be represented as frame bytes. Middleware that mutates
    /// [`frame_mut`](Self::frame_mut) therefore does not alter a deferred body,
    /// and middleware that constructs a fresh `ServiceResponse` discards it.
    deferred: Option<DeferredBody>,
}

impl Clone for ServiceResponse {
    /// Clone the response, sharing any deferred body with the original.
    fn clone(&self) -> Self {
        Self {
            inner: FrameContainer::new(self.inner.frame().clone()),
            correlation_id: self.correlation_id,
            deferred: self.deferred.clone(),
        }
    }
}

impl PartialEq for ServiceResponse {
    /// Compare the visible frame bytes and correlation metadata.
    ///
    /// Deferred bodies are excluded because they may own live channels and are
    /// consumed exactly once.
    fn eq(&self, other: &Self) -> bool {
        self.inner.frame() == other.inner.frame() && self.correlation_id == other.correlation_id
    }
}

impl std::fmt::Debug for ServiceResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceResponse")
            .field("frame", &self.inner.frame())
            .field("correlation_id", &self.correlation_id)
            .field("has_deferred_body", &self.deferred.is_some())
            .finish()
    }
}

impl ServiceResponse {
    /// Create a new [`ServiceResponse`] containing the given frame bytes.
    #[must_use]
    pub const fn new(frame: Vec<u8>, correlation_id: Option<u64>) -> Self {
        Self {
            inner: FrameContainer::new(frame),
            correlation_id,
            deferred: None,
        }
    }

    /// Create a response carrying a route handler's explicit response body.
    ///
    /// The frame bytes remain available to middleware, but the deferred body
    /// is what the connection streams; byte mutations do not apply to it.
    #[must_use]
    pub(crate) const fn with_deferred(
        frame: Vec<u8>,
        correlation_id: Option<u64>,
        deferred: DeferredBody,
    ) -> Self {
        Self {
            inner: FrameContainer::new(frame),
            correlation_id,
            deferred: Some(deferred),
        }
    }

    /// Take the deferred response body, if one is still present.
    ///
    /// The slot is shared between clones and can be taken once; later calls
    /// return `None`.
    pub(crate) fn take_deferred(&self) -> Option<Response<Envelope, ()>> {
        let slot = self.deferred.as_ref()?;
        slot.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// Borrow the inner frame bytes.
    #[must_use]
    pub const fn frame(&self) -> &[u8] { self.inner.frame().as_slice() }

    /// Mutable access to the response frame bytes.
    #[must_use]
    pub const fn frame_mut(&mut self) -> &mut Vec<u8> { self.inner.frame_mut() }

    /// Return the correlation identifier associated with this response, if any.
    #[must_use]
    pub const fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    /// Set or clear the correlation identifier.
    #[must_use]
    pub const fn set_correlation_id(&mut self, correlation_id: Option<u64>) -> &mut Self {
        self.correlation_id = correlation_id;
        self
    }

    /// Consume the response, yielding the raw frame bytes.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> { self.inner.into_inner() }
}
