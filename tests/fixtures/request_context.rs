//! `RequestContextWorld` fixture for request context BDD tests.
//!
//! Provides a self-contained world that creates a [`MessageRequest`],
//! optionally inserts a [`RequestContext`] into its shared state, and
//! supports extraction and verification in step definitions.

use rstest::fixture;
use wireframe::{
    context::{CorrelationId, MissingTenantError, RequestContext},
    extractor::{FromMessageRequest, MessageRequest, Payload},
    tenant::{TenantId, UserId},
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

/// Test world for request context BDD scenarios.
pub struct RequestContextWorld {
    request: MessageRequest,
    extracted: Option<RequestContext>,
    tenant_error: Option<MissingTenantError>,
}

impl std::fmt::Debug for RequestContextWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestContextWorld")
            .field("request", &"..")
            .field("extracted", &self.extracted)
            .field("tenant_error", &self.tenant_error)
            .finish()
    }
}

impl Default for RequestContextWorld {
    fn default() -> Self {
        Self {
            request: MessageRequest::new(),
            extracted: None,
            tenant_error: None,
        }
    }
}

// rustfmt collapses simple fixtures into one line, which triggers
// unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn request_context_world() -> RequestContextWorld {
    RequestContextWorld::default()
}

impl RequestContextWorld {
    /// Insert a [`RequestContext`] into the message request's shared
    /// state.
    pub fn register_context(&self, ctx: RequestContext) { self.request.insert_state(ctx); }

    /// Extract the [`RequestContext`] from the message request using
    /// the standard extractor mechanism.
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails (should be infallible).
    pub fn extract(&mut self) -> TestResult {
        let ctx = RequestContext::from_message_request(&self.request, &mut Payload::default())
            .map_err(|e| format!("extraction failed: {e}"))?;
        self.extracted = Some(ctx);
        Ok(())
    }

    /// Call `require_tenant_id` on the extracted (or default) context
    /// and store any error.
    pub fn call_require_tenant_id(&mut self) {
        let ctx = self.extracted.clone().unwrap_or_default();
        if let Err(e) = ctx.require_tenant_id() {
            self.tenant_error = Some(e);
        }
    }

    /// Return the extracted tenant identifier, if present.
    #[must_use]
    pub fn extracted_tenant_id(&self) -> Option<TenantId> {
        self.extracted.as_ref().and_then(RequestContext::tenant_id)
    }

    /// Return the extracted user identifier, if present.
    #[must_use]
    pub fn extracted_user_id(&self) -> Option<UserId> {
        self.extracted.as_ref().and_then(RequestContext::user_id)
    }

    /// Return the extracted correlation identifier, if present.
    #[must_use]
    pub fn extracted_correlation_id(&self) -> Option<CorrelationId> {
        self.extracted
            .as_ref()
            .and_then(RequestContext::correlation_id)
    }

    /// Return whether a missing tenant error was captured.
    #[must_use]
    pub fn has_tenant_error(&self) -> bool { self.tenant_error.is_some() }

    /// Return whether all extracted identifiers are absent.
    #[must_use]
    pub fn all_identifiers_absent(&self) -> bool {
        self.extracted.as_ref().is_some_and(|ctx| {
            ctx.tenant_id().is_none()
                && ctx.user_id().is_none()
                && ctx.session_id().is_none()
                && ctx.correlation_id().is_none()
                && ctx.causation_id().is_none()
        })
    }
}
