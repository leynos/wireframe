//! Cross-cutting request context struct and associated error types.

use super::{CausationId, CorrelationId, SessionId};
use crate::tenant::{TenantId, UserId};

/// Cross-cutting request context carrying tenant, user, and tracing
/// identifiers.
///
/// `RequestContext` is designed to be inserted into the
/// [`AppDataStore`](crate::app_data_store::AppDataStore) per-request
/// and extracted by handlers via the
/// [`FromMessageRequest`](crate::extractor::FromMessageRequest) trait.
///
/// All fields are optional to support both tenant-aware and
/// tenant-agnostic operations. For tenant-owned operations, use
/// [`Self::require_tenant_id`] to enforce presence at the call site.
///
/// # Examples
///
/// ```rust
/// use wireframe::{
///     context::{CorrelationId, RequestContext, SessionId},
///     tenant::{TenantId, UserId},
/// };
///
/// let ctx = RequestContext::new()
///     .with_tenant_id(TenantId::new(1))
///     .with_user_id(UserId::new(42))
///     .with_correlation_id(CorrelationId::new(100))
///     .with_session_id(SessionId::new(7));
///
/// assert_eq!(ctx.tenant_id(), Some(TenantId::new(1)));
/// assert_eq!(ctx.user_id(), Some(UserId::new(42)));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "fields are domain identifiers; the _id suffix is the domain name, not a naming defect"
)]
pub struct RequestContext {
    tenant_id: Option<TenantId>,
    user_id: Option<UserId>,
    session_id: Option<SessionId>,
    correlation_id: Option<CorrelationId>,
    causation_id: Option<CausationId>,
}

impl RequestContext {
    /// Create a new empty request context with all fields set to `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::RequestContext;
    ///
    /// let ctx = RequestContext::new();
    /// assert!(ctx.tenant_id().is_none());
    /// ```
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Set the tenant identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{context::RequestContext, tenant::TenantId};
    ///
    /// let ctx = RequestContext::new().with_tenant_id(TenantId::new(5));
    /// assert_eq!(ctx.tenant_id(), Some(TenantId::new(5)));
    /// ```
    #[must_use]
    pub fn with_tenant_id(mut self, id: TenantId) -> Self {
        self.tenant_id = Some(id);
        self
    }

    /// Set the user identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{context::RequestContext, tenant::UserId};
    ///
    /// let ctx = RequestContext::new().with_user_id(UserId::new(10));
    /// assert_eq!(ctx.user_id(), Some(UserId::new(10)));
    /// ```
    #[must_use]
    pub fn with_user_id(mut self, id: UserId) -> Self {
        self.user_id = Some(id);
        self
    }

    /// Set the session identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::{RequestContext, SessionId};
    ///
    /// let ctx = RequestContext::new().with_session_id(SessionId::new(3));
    /// assert_eq!(ctx.session_id(), Some(SessionId::new(3)));
    /// ```
    #[must_use]
    pub fn with_session_id(mut self, id: SessionId) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Set the correlation identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::{CorrelationId, RequestContext};
    ///
    /// let ctx = RequestContext::new().with_correlation_id(CorrelationId::new(99));
    /// assert_eq!(ctx.correlation_id(), Some(CorrelationId::new(99)));
    /// ```
    #[must_use]
    pub fn with_correlation_id(mut self, id: CorrelationId) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set the causation identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::{CausationId, RequestContext};
    ///
    /// let ctx = RequestContext::new().with_causation_id(CausationId::new(77));
    /// assert_eq!(ctx.causation_id(), Some(CausationId::new(77)));
    /// ```
    #[must_use]
    pub fn with_causation_id(mut self, id: CausationId) -> Self {
        self.causation_id = Some(id);
        self
    }

    /// Return the tenant identifier, if set.
    #[must_use]
    pub fn tenant_id(&self) -> Option<TenantId> { self.tenant_id }

    /// Return the user identifier, if set.
    #[must_use]
    pub fn user_id(&self) -> Option<UserId> { self.user_id }

    /// Return the session identifier, if set.
    #[must_use]
    pub fn session_id(&self) -> Option<SessionId> { self.session_id }

    /// Return the correlation identifier, if set.
    #[must_use]
    pub fn correlation_id(&self) -> Option<CorrelationId> { self.correlation_id }

    /// Return the causation identifier, if set.
    #[must_use]
    pub fn causation_id(&self) -> Option<CausationId> { self.causation_id }

    /// Require a tenant identifier, returning an error if absent.
    ///
    /// Use this in service and repository signatures that mandate
    /// tenant-aware operations.
    ///
    /// # Errors
    ///
    /// Returns [`MissingTenantError`] if no tenant identifier is present.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{context::RequestContext, tenant::TenantId};
    ///
    /// let ctx = RequestContext::new().with_tenant_id(TenantId::new(1));
    /// assert!(ctx.require_tenant_id().is_ok());
    ///
    /// let empty = RequestContext::new();
    /// assert!(empty.require_tenant_id().is_err());
    /// ```
    pub fn require_tenant_id(&self) -> Result<TenantId, MissingTenantError> {
        self.tenant_id.ok_or(MissingTenantError)
    }

    /// Require a user identifier, returning an error if absent.
    ///
    /// # Errors
    ///
    /// Returns [`MissingUserError`] if no user identifier is present.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{context::RequestContext, tenant::UserId};
    ///
    /// let ctx = RequestContext::new().with_user_id(UserId::new(42));
    /// assert!(ctx.require_user_id().is_ok());
    ///
    /// let empty = RequestContext::new();
    /// assert!(empty.require_user_id().is_err());
    /// ```
    pub fn require_user_id(&self) -> Result<UserId, MissingUserError> {
        self.user_id.ok_or(MissingUserError)
    }

    /// Create a context from a raw correlation identifier.
    ///
    /// This bridges the existing `Option<u64>` correlation pattern
    /// (used by [`CorrelatableFrame`](crate::correlation::CorrelatableFrame))
    /// into the typed `RequestContext`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::{CorrelationId, RequestContext};
    ///
    /// let ctx = RequestContext::from_raw_correlation(Some(42));
    /// assert_eq!(ctx.correlation_id(), Some(CorrelationId::new(42)));
    ///
    /// let empty = RequestContext::from_raw_correlation(None);
    /// assert!(empty.correlation_id().is_none());
    /// ```
    #[must_use]
    pub fn from_raw_correlation(correlation_id: Option<u64>) -> Self {
        Self {
            correlation_id: correlation_id.map(CorrelationId::from),
            ..Self::default()
        }
    }
}

/// Error indicating a required tenant identifier is missing from the
/// request context.
#[derive(Debug, thiserror::Error)]
#[error("tenant identifier is required but not present in request context")]
pub struct MissingTenantError;

/// Error indicating a required user identifier is missing from the
/// request context.
#[derive(Debug, thiserror::Error)]
#[error("user identifier is required but not present in request context")]
pub struct MissingUserError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_has_all_none() {
        let ctx = RequestContext::new();
        assert!(ctx.tenant_id().is_none());
        assert!(ctx.user_id().is_none());
        assert!(ctx.session_id().is_none());
        assert!(ctx.correlation_id().is_none());
        assert!(ctx.causation_id().is_none());
    }

    #[test]
    fn builder_chaining_sets_fields() {
        let ctx = RequestContext::new()
            .with_tenant_id(TenantId::new(1))
            .with_user_id(UserId::new(2))
            .with_session_id(SessionId::new(3))
            .with_correlation_id(CorrelationId::new(4))
            .with_causation_id(CausationId::new(5));

        assert_eq!(ctx.tenant_id(), Some(TenantId::new(1)));
        assert_eq!(ctx.user_id(), Some(UserId::new(2)));
        assert_eq!(ctx.session_id(), Some(SessionId::new(3)));
        assert_eq!(ctx.correlation_id(), Some(CorrelationId::new(4)));
        assert_eq!(ctx.causation_id(), Some(CausationId::new(5)));
    }

    #[test]
    fn require_tenant_id_returns_ok_when_present() {
        let ctx = RequestContext::new().with_tenant_id(TenantId::new(10));
        assert_eq!(ctx.require_tenant_id().ok(), Some(TenantId::new(10)));
    }

    #[test]
    fn require_tenant_id_returns_err_when_absent() {
        let ctx = RequestContext::new();
        assert!(ctx.require_tenant_id().is_err());
    }

    #[test]
    fn require_user_id_returns_ok_when_present() {
        let ctx = RequestContext::new().with_user_id(UserId::new(20));
        assert_eq!(ctx.require_user_id().ok(), Some(UserId::new(20)));
    }

    #[test]
    fn require_user_id_returns_err_when_absent() {
        let ctx = RequestContext::new();
        assert!(ctx.require_user_id().is_err());
    }

    #[test]
    fn from_raw_correlation_creates_context() {
        let ctx = RequestContext::from_raw_correlation(Some(42));
        assert_eq!(ctx.correlation_id(), Some(CorrelationId::new(42)));
        assert!(ctx.tenant_id().is_none());
        assert!(ctx.user_id().is_none());
    }

    #[test]
    fn from_raw_correlation_none_creates_empty_context() {
        let ctx = RequestContext::from_raw_correlation(None);
        assert!(ctx.correlation_id().is_none());
    }

    #[test]
    fn clone_produces_equal_context() {
        let ctx = RequestContext::new()
            .with_tenant_id(TenantId::new(1))
            .with_user_id(UserId::new(2));
        assert_eq!(ctx.clone(), ctx);
    }

    #[test]
    fn missing_tenant_error_display() {
        let err = MissingTenantError;
        let msg = format!("{err}");
        assert!(msg.contains("tenant identifier is required"));
    }

    #[test]
    fn missing_user_error_display() {
        let err = MissingUserError;
        let msg = format!("{err}");
        assert!(msg.contains("user identifier is required"));
    }
}
