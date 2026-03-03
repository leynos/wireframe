//! Cross-cutting request context for tenant-aware operations.
//!
//! [`RequestContext`] carries tenant, user, session, correlation, and
//! causation identifiers through the request pipeline. It replaces
//! ad-hoc audit context usage with a unified, extractable context
//! object.

mod identifiers;
mod request_context;

pub use identifiers::{CausationId, CorrelationId, SessionId};
pub use request_context::{MissingTenantError, MissingUserError, RequestContext};
