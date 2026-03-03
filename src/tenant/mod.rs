//! Tenant domain primitives for multi-tenancy support.
//!
//! This module provides the core identity types used to model tenancy
//! within wireframe applications. The initial model uses one owning
//! user per tenant, preserving a separate user-versus-tenant identity
//! model for future team and organisation tenants.
//!
//! # Types
//!
//! - [`TenantId`] — unique numeric tenant identifier.
//! - [`TenantSlug`] — human-readable, URL-friendly tenant name.
//! - [`UserId`] — unique numeric user identifier (distinct from tenant).
//! - [`Tenant`] — aggregate combining identity, slug, and owning user.

mod primitives;

pub use primitives::{Tenant, TenantId, TenantSlug, UserId};
