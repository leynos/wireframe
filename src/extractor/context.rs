//! Extractor for cross-cutting request context.
//!
//! Implements [`FromMessageRequest`] for [`RequestContext`], making
//! the context available as a handler parameter via the standard
//! extractor mechanism.

use super::{FromMessageRequest, MessageRequest, Payload};
use crate::context::RequestContext;

impl FromMessageRequest for RequestContext {
    type Error = std::convert::Infallible;

    /// Extract the [`RequestContext`] from the request's shared state.
    ///
    /// Returns a default (all-`None`) context if none was registered,
    /// making this extraction infallible. Handlers that require
    /// specific fields should call
    /// [`RequestContext::require_tenant_id`] or
    /// [`RequestContext::require_user_id`] after extraction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     context::RequestContext,
    ///     extractor::{FromMessageRequest, MessageRequest, Payload},
    ///     tenant::TenantId,
    /// };
    ///
    /// let req = MessageRequest::new();
    /// req.insert_state(RequestContext::new().with_tenant_id(TenantId::new(1)));
    /// let ctx = RequestContext::from_message_request(&req, &mut Payload::default())
    ///     .expect("extraction is infallible");
    /// assert_eq!(ctx.tenant_id(), Some(TenantId::new(1)));
    /// ```
    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        Ok(req
            .state::<RequestContext>()
            .map(|s| (*s).clone())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        context::RequestContext,
        extractor::{FromMessageRequest, MessageRequest, Payload},
        tenant::TenantId,
    };

    #[test]
    fn extract_returns_default_when_no_context_registered() {
        let req = MessageRequest::new();
        let ctx = RequestContext::from_message_request(&req, &mut Payload::default())
            .expect("extraction is infallible");
        assert_eq!(ctx, RequestContext::new());
    }

    #[test]
    fn extract_returns_stored_context() {
        let req = MessageRequest::new();
        let expected = RequestContext::new().with_tenant_id(TenantId::new(42));
        req.insert_state(expected.clone());

        let ctx = RequestContext::from_message_request(&req, &mut Payload::default())
            .expect("extraction is infallible");
        assert_eq!(ctx, expected);
    }
}
