//! Cross-cutting identifier newtypes for request tracing.

use std::fmt;

/// Correlation identifier linking related operations.
///
/// Wraps the raw `u64` correlation pattern used throughout the
/// framework (see [`crate::correlation::CorrelatableFrame`]) in a
/// domain newtype. Use [`From<u64>`] and [`Self::as_u64`] to bridge
/// between the newtype and existing raw-integer APIs.
///
/// # Examples
///
/// ```rust
/// use wireframe::context::CorrelationId;
///
/// let id = CorrelationId::new(100);
/// assert_eq!(id.as_u64(), 100);
/// assert_eq!(format!("{id}"), "CorrelationId(100)");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CorrelationId(u64);

impl From<u64> for CorrelationId {
    fn from(value: u64) -> Self { Self(value) }
}

impl CorrelationId {
    /// Create a new [`CorrelationId`] with the provided value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::CorrelationId;
    ///
    /// let id = CorrelationId::new(1);
    /// assert_eq!(id.as_u64(), 1);
    /// ```
    #[must_use]
    pub fn new(id: u64) -> Self { Self(id) }

    /// Return the inner `u64` representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::CorrelationId;
    ///
    /// let id = CorrelationId::from(7u64);
    /// assert_eq!(id.as_u64(), 7);
    /// ```
    #[must_use]
    pub fn as_u64(&self) -> u64 { self.0 }
}

impl fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CorrelationId({})", self.0)
    }
}

/// Causation identifier tracing the operation that triggered this one.
///
/// In an event-driven system the causation identifier links an effect
/// back to its direct cause, forming a causal chain that can be
/// traversed for debugging and auditing.
///
/// # Examples
///
/// ```rust
/// use wireframe::context::CausationId;
///
/// let id = CausationId::new(55);
/// assert_eq!(id.as_u64(), 55);
/// assert_eq!(format!("{id}"), "CausationId(55)");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CausationId(u64);

impl From<u64> for CausationId {
    fn from(value: u64) -> Self { Self(value) }
}

impl CausationId {
    /// Create a new [`CausationId`] with the provided value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::CausationId;
    ///
    /// let id = CausationId::new(1);
    /// assert_eq!(id.as_u64(), 1);
    /// ```
    #[must_use]
    pub fn new(id: u64) -> Self { Self(id) }

    /// Return the inner `u64` representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::CausationId;
    ///
    /// let id = CausationId::from(3u64);
    /// assert_eq!(id.as_u64(), 3);
    /// ```
    #[must_use]
    pub fn as_u64(&self) -> u64 { self.0 }
}

impl fmt::Display for CausationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CausationId({})", self.0)
    }
}

/// Session identifier linking operations within the same logical session.
///
/// A session may span multiple connections (for example after a
/// reconnection) or a connection may serve multiple sessions.
/// `SessionId` is intentionally distinct from
/// [`ConnectionId`](crate::session::ConnectionId), which is a
/// transport-level concern.
///
/// # Examples
///
/// ```rust
/// use wireframe::context::SessionId;
///
/// let id = SessionId::new(8);
/// assert_eq!(id.as_u64(), 8);
/// assert_eq!(format!("{id}"), "SessionId(8)");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionId(u64);

impl From<u64> for SessionId {
    fn from(value: u64) -> Self { Self(value) }
}

impl SessionId {
    /// Create a new [`SessionId`] with the provided value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::SessionId;
    ///
    /// let id = SessionId::new(1);
    /// assert_eq!(id.as_u64(), 1);
    /// ```
    #[must_use]
    pub fn new(id: u64) -> Self { Self(id) }

    /// Return the inner `u64` representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::context::SessionId;
    ///
    /// let id = SessionId::from(9u64);
    /// assert_eq!(id.as_u64(), 9);
    /// ```
    #[must_use]
    pub fn as_u64(&self) -> u64 { self.0 }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "SessionId({})", self.0) }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    macro_rules! test_id_equality_and_hash {
        ($name:ident, $ty:ty, $val_a:expr, $val_b:expr) => {
            #[test]
            fn $name() {
                let a = <$ty>::new($val_a);
                let b = <$ty>::new($val_a);
                let c = <$ty>::new($val_b);
                assert_eq!(a, b);
                assert_ne!(a, c);

                let mut set = HashSet::new();
                set.insert(a);
                assert!(set.contains(&b));
                assert!(!set.contains(&c));
            }
        };
    }

    #[test]
    fn correlation_id_round_trip() {
        let id = CorrelationId::new(42);
        assert_eq!(id.as_u64(), 42);
        assert_eq!(CorrelationId::from(42u64), id);
    }

    #[test]
    fn correlation_id_display() {
        assert_eq!(format!("{}", CorrelationId::new(7)), "CorrelationId(7)");
    }

    test_id_equality_and_hash!(correlation_id_equality_and_hash, CorrelationId, 1, 2);

    #[test]
    fn causation_id_round_trip() {
        let id = CausationId::new(55);
        assert_eq!(id.as_u64(), 55);
        assert_eq!(CausationId::from(55u64), id);
    }

    #[test]
    fn causation_id_display() {
        assert_eq!(format!("{}", CausationId::new(3)), "CausationId(3)");
    }

    test_id_equality_and_hash!(causation_id_equality_and_hash, CausationId, 10, 20);

    #[test]
    fn session_id_round_trip() {
        let id = SessionId::new(8);
        assert_eq!(id.as_u64(), 8);
        assert_eq!(SessionId::from(8u64), id);
    }

    #[test]
    fn session_id_display() {
        assert_eq!(format!("{}", SessionId::new(9)), "SessionId(9)");
    }

    test_id_equality_and_hash!(session_id_equality_and_hash, SessionId, 5, 6);
}
