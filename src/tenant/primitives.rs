//! Domain primitive types for tenant and user identity.

use std::fmt;

/// Unique identifier for a tenant.
///
/// Backed by a `u64` to match the binary protocol's identifier
/// conventions (see [`crate::session::ConnectionId`]). Applications
/// needing UUID-based identifiers can wrap this type in their own
/// domain layer.
///
/// # Examples
///
/// ```rust
/// use wireframe::tenant::TenantId;
///
/// let id = TenantId::new(42);
/// assert_eq!(id.as_u64(), 42);
/// assert_eq!(format!("{id}"), "TenantId(42)");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TenantId(u64);

impl From<u64> for TenantId {
    fn from(value: u64) -> Self { Self(value) }
}

impl TenantId {
    /// Create a new [`TenantId`] with the provided value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::TenantId;
    ///
    /// let id = TenantId::new(1);
    /// assert_eq!(id.as_u64(), 1);
    /// ```
    #[must_use]
    pub fn new(id: u64) -> Self { Self(id) }

    /// Return the inner `u64` representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::TenantId;
    ///
    /// let id = TenantId::from(7u64);
    /// assert_eq!(id.as_u64(), 7);
    /// ```
    #[must_use]
    pub fn as_u64(&self) -> u64 { self.0 }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "TenantId({})", self.0) }
}

/// Human-readable tenant slug for URL-friendly identification.
///
/// A slug is a short, lowercase, hyphenated string suitable for use in
/// URLs and configuration keys (for example `acme-corp`). No validation
/// is applied at construction time — callers are responsible for
/// enforcing slug format rules appropriate to their domain.
///
/// # Examples
///
/// ```rust
/// use wireframe::tenant::TenantSlug;
///
/// let slug = TenantSlug::new("acme-corp");
/// assert_eq!(slug.as_str(), "acme-corp");
/// assert_eq!(format!("{slug}"), "acme-corp");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TenantSlug(String);

impl TenantSlug {
    /// Create a new [`TenantSlug`] from any value convertible to `String`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::TenantSlug;
    ///
    /// let slug = TenantSlug::new("my-tenant");
    /// assert_eq!(slug.as_str(), "my-tenant");
    /// ```
    #[must_use]
    pub fn new(slug: impl Into<String>) -> Self { Self(slug.into()) }

    /// Return the slug as a string slice.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::TenantSlug;
    ///
    /// let slug = TenantSlug::new("demo");
    /// assert_eq!(slug.as_str(), "demo");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl From<&str> for TenantSlug {
    fn from(value: &str) -> Self { Self(value.to_owned()) }
}

impl From<String> for TenantSlug {
    fn from(value: String) -> Self { Self(value) }
}

impl AsRef<str> for TenantSlug {
    fn as_ref(&self) -> &str { &self.0 }
}

impl fmt::Display for TenantSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

/// Unique identifier for a user.
///
/// `UserId` is intentionally distinct from [`TenantId`] to preserve a
/// separate user-versus-tenant identity model. The initial tenancy
/// model uses one owning user per tenant, but future iterations may
/// support team and organisation tenants with multiple members.
///
/// # Examples
///
/// ```rust
/// use wireframe::tenant::UserId;
///
/// let id = UserId::new(99);
/// assert_eq!(id.as_u64(), 99);
/// assert_eq!(format!("{id}"), "UserId(99)");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct UserId(u64);

impl From<u64> for UserId {
    fn from(value: u64) -> Self { Self(value) }
}

impl UserId {
    /// Create a new [`UserId`] with the provided value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::UserId;
    ///
    /// let id = UserId::new(1);
    /// assert_eq!(id.as_u64(), 1);
    /// ```
    #[must_use]
    pub fn new(id: u64) -> Self { Self(id) }

    /// Return the inner `u64` representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::UserId;
    ///
    /// let id = UserId::from(5u64);
    /// assert_eq!(id.as_u64(), 5);
    /// ```
    #[must_use]
    pub fn as_u64(&self) -> u64 { self.0 }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "UserId({})", self.0) }
}

/// A tenant with its identifying metadata and owning user.
///
/// The initial tenancy model uses one owning user per tenant. Future
/// iterations may support team and organisation tenants with multiple
/// members, but the type system preserves the user-versus-tenant
/// distinction from the outset.
///
/// # Examples
///
/// ```rust
/// use wireframe::tenant::{Tenant, TenantId, TenantSlug, UserId};
///
/// let tenant = Tenant::new(
///     TenantId::new(1),
///     TenantSlug::new("acme-corp"),
///     UserId::new(100),
/// );
/// assert_eq!(tenant.id(), TenantId::new(1));
/// assert_eq!(tenant.slug().as_str(), "acme-corp");
/// assert_eq!(tenant.owner(), UserId::new(100));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tenant {
    id: TenantId,
    slug: TenantSlug,
    owner: UserId,
}

impl Tenant {
    /// Create a new [`Tenant`] with the given identity, slug, and owner.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::{Tenant, TenantId, TenantSlug, UserId};
    ///
    /// let t = Tenant::new(TenantId::new(10), TenantSlug::new("demo"), UserId::new(1));
    /// assert_eq!(t.id().as_u64(), 10);
    /// ```
    #[must_use]
    pub fn new(id: TenantId, slug: TenantSlug, owner: UserId) -> Self { Self { id, slug, owner } }

    /// Return the tenant's unique identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::{Tenant, TenantId, TenantSlug, UserId};
    ///
    /// let t = Tenant::new(TenantId::new(5), TenantSlug::new("x"), UserId::new(1));
    /// assert_eq!(t.id(), TenantId::new(5));
    /// ```
    #[must_use]
    pub fn id(&self) -> TenantId { self.id }

    /// Return a reference to the tenant's slug.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::{Tenant, TenantId, TenantSlug, UserId};
    ///
    /// let t = Tenant::new(TenantId::new(1), TenantSlug::new("slug"), UserId::new(1));
    /// assert_eq!(t.slug().as_str(), "slug");
    /// ```
    #[must_use]
    pub fn slug(&self) -> &TenantSlug { &self.slug }

    /// Return the owning user's identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::tenant::{Tenant, TenantId, TenantSlug, UserId};
    ///
    /// let t = Tenant::new(TenantId::new(1), TenantSlug::new("x"), UserId::new(42));
    /// assert_eq!(t.owner(), UserId::new(42));
    /// ```
    #[must_use]
    pub fn owner(&self) -> UserId { self.owner }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn tenant_id_round_trip() {
        let id = TenantId::new(42);
        assert_eq!(id.as_u64(), 42);
        assert_eq!(TenantId::from(42u64), id);
    }

    #[test]
    fn tenant_id_display() {
        assert_eq!(format!("{}", TenantId::new(7)), "TenantId(7)");
    }

    #[test]
    fn tenant_id_equality_and_hash() {
        let a = TenantId::new(1);
        let b = TenantId::new(1);
        let c = TenantId::new(2);
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn tenant_slug_from_str() {
        let slug = TenantSlug::from("hello");
        assert_eq!(slug.as_str(), "hello");
    }

    #[test]
    fn tenant_slug_from_string() {
        let slug = TenantSlug::from(String::from("world"));
        assert_eq!(slug.as_str(), "world");
    }

    #[test]
    fn tenant_slug_as_ref() {
        let slug = TenantSlug::new("test");
        let s: &str = slug.as_ref();
        assert_eq!(s, "test");
    }

    #[test]
    fn tenant_slug_display() {
        assert_eq!(format!("{}", TenantSlug::new("acme")), "acme");
    }

    #[test]
    fn tenant_slug_equality() {
        assert_eq!(TenantSlug::new("a"), TenantSlug::new("a"));
        assert_ne!(TenantSlug::new("a"), TenantSlug::new("b"));
    }

    #[test]
    fn user_id_round_trip() {
        let id = UserId::new(99);
        assert_eq!(id.as_u64(), 99);
        assert_eq!(UserId::from(99u64), id);
    }

    #[test]
    fn user_id_display() {
        assert_eq!(format!("{}", UserId::new(5)), "UserId(5)");
    }

    #[test]
    fn user_id_equality_and_hash() {
        let a = UserId::new(10);
        let b = UserId::new(10);
        let c = UserId::new(20);
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn tenant_construction_and_accessors() {
        let tenant = Tenant::new(
            TenantId::new(1),
            TenantSlug::new("acme-corp"),
            UserId::new(100),
        );
        assert_eq!(tenant.id(), TenantId::new(1));
        assert_eq!(tenant.slug().as_str(), "acme-corp");
        assert_eq!(tenant.owner(), UserId::new(100));
    }

    #[test]
    fn tenant_equality() {
        let a = Tenant::new(TenantId::new(1), TenantSlug::new("x"), UserId::new(1));
        let b = Tenant::new(TenantId::new(1), TenantSlug::new("x"), UserId::new(1));
        let c = Tenant::new(TenantId::new(2), TenantSlug::new("x"), UserId::new(1));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn tenant_id_and_user_id_are_distinct_types() {
        // Compile-time: TenantId and UserId are not interchangeable.
        let tid = TenantId::new(1);
        let uid = UserId::new(1);
        // Same inner value, but distinct types.
        assert_eq!(tid.as_u64(), uid.as_u64());
        // No From<UserId> for TenantId or vice versa exists.
    }
}
