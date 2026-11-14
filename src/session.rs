//! Registry of active connection push handles.
//!
//! `SessionRegistry` stores non-owning weak references to [`PushHandle`]s,
//! allowing asynchronous tasks to send frames to live connections without
//! preventing their cleanup. Dead entries can be pruned opportunistically or
//! lazily at lookup time.
use std::sync::{
    Arc,
    Weak,
    atomic::{AtomicU64, Ordering},
};

use dashmap::DashMap;

use crate::push::{FrameLike, PushHandle, PushHandleInner};

/// Identifier assigned to a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

impl From<u64> for ConnectionId {
    fn from(value: u64) -> Self { Self(value) }
}

impl ConnectionId {
    /// Create a new [`ConnectionId`] with the provided value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::session::ConnectionId;
    ///
    /// let id = ConnectionId::new(42);
    /// assert_eq!(id.as_u64(), 42);
    /// ```
    #[must_use]
    pub const fn new(id: u64) -> Self { Self(id) }

    /// Return the inner `u64` representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::session::ConnectionId;
    ///
    /// let id = ConnectionId::from(7u64);
    /// assert_eq!(id.as_u64(), 7);
    /// ```
    #[must_use]
    pub const fn as_u64(&self) -> u64 { self.0 }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConnectionId({})", self.0)
    }
}

/// Concurrent registry of push handles keyed by [`ConnectionId`].
///
/// The registry also owns the source of connection identifiers. Identifiers
/// start at one and increase monotonically, so a value freed by a closed
/// connection is never reused for a later one.
pub struct SessionRegistry<F> {
    /// Weak references to the live push handles, keyed by connection.
    entries: DashMap<ConnectionId, Weak<PushHandleInner<F>>>,
    /// Next identifier handed to a connection.
    next_id: AtomicU64,
}

impl<F> Default for SessionRegistry<F> {
    fn default() -> Self { Self::new() }
}

impl<F> SessionRegistry<F> {
    /// Create an empty registry.
    ///
    /// The first identifier allocated by [`Self::next_connection_id`] is `1`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::session::{ConnectionId, SessionRegistry};
    ///
    /// let registry = SessionRegistry::<u8>::new();
    /// assert_eq!(registry.next_connection_id(), ConnectionId::new(1));
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocate the next connection identifier.
    ///
    /// Identifiers are unique for the lifetime of the registry and are never
    /// reused. Callers should allocate one per accepted connection and store
    /// it alongside the connection's push handle.
    ///
    /// # Panics
    ///
    /// Panics when the `u64` identifier space is exhausted. Reaching that
    /// point requires allocating every possible identifier, so the panic
    /// reports an unrecoverable configuration error rather than wrapping and
    /// handing out a duplicate.
    #[must_use]
    pub fn next_connection_id(&self) -> ConnectionId {
        let updated = self
            .next_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            });
        let Ok(id) = updated else {
            panic!("connection identifier space exhausted");
        };
        ConnectionId::new(id)
    }
}

impl<F: FrameLike> SessionRegistry<F> {
    /// Retain live entries and collect data from each upgraded handle.
    fn retain_and_collect<T>(
        &self,
        mut map: impl FnMut(ConnectionId, Arc<PushHandleInner<F>>) -> T,
    ) -> Vec<T> {
        let mut out = Vec::with_capacity(self.entries.len());
        self.entries.retain(|id, weak| {
            weak.upgrade().is_some_and(|inner| {
                out.push(map(*id, inner));
                true
            })
        });
        out
    }

    /// Retrieve a [`PushHandle`] for `id` if the connection is still alive.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     push::PushQueues,
    ///     session::{ConnectionId, SessionRegistry},
    /// };
    ///
    /// let (_queues, handle) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let registry = SessionRegistry::default();
    /// let id = ConnectionId::new(1);
    /// registry.insert(id, &handle);
    /// assert!(registry.get(&id).is_some());
    /// ```
    pub fn get(&self, id: &ConnectionId) -> Option<PushHandle<F>> {
        let guard = self.entries.get(id);
        let handle = guard.as_ref().and_then(|weak| weak.upgrade());
        drop(guard);
        if handle.is_none() {
            self.entries
                .remove_if(id, |_, weak| weak.strong_count() == 0);
        }
        handle.map(PushHandle::from_arc)
    }

    /// Insert a handle for a newly established connection.
    ///
    /// An existing entry for the same identifier is replaced. A server
    /// connection inserts its own handle under an identifier from
    /// [`next_connection_id`](Self::next_connection_id), which does not skip
    /// identifiers already present, so an application that still inserts
    /// entries manually must not reuse identifiers the registry allocates:
    /// its entry would be replaced and pushes to that identifier would reach
    /// the server connection instead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     push::PushQueues,
    ///     session::{ConnectionId, SessionRegistry},
    /// };
    ///
    /// let (_queues, handle) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let registry = SessionRegistry::default();
    /// let id = ConnectionId::new(2);
    /// assert!(registry.get(&id).is_none());
    /// registry.insert(id, &handle);
    /// assert!(registry.get(&id).is_some());
    /// ```
    pub fn insert(&self, id: ConnectionId, handle: &PushHandle<F>) {
        self.entries.insert(id, handle.downgrade());
    }

    /// Remove a handle, typically on connection teardown.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     push::PushQueues,
    ///     session::{ConnectionId, SessionRegistry},
    /// };
    ///
    /// let (_queues, handle) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let registry = SessionRegistry::default();
    /// let id = ConnectionId::new(3);
    /// registry.insert(id, &handle);
    /// assert!(registry.get(&id).is_some());
    /// registry.remove(&id);
    /// assert!(registry.get(&id).is_none());
    /// ```
    pub fn remove(&self, id: &ConnectionId) { self.entries.remove(id); }

    /// Remove all stale weak references without returning any handles.
    ///
    /// `DashMap::retain` acquires per-bucket write locks, so other operations
    /// may contend briefly while the registry is pruned.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     push::PushQueues,
    ///     session::{ConnectionId, SessionRegistry},
    /// };
    ///
    /// let registry = SessionRegistry::default();
    /// let (_queues, handle) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let id = ConnectionId::new(4);
    /// registry.insert(id, &handle);
    /// drop(handle);
    /// registry.prune();
    /// assert!(registry.get(&id).is_none());
    /// ```
    pub fn prune(&self) { self.entries.retain(|_, weak| weak.strong_count() > 0); }

    /// Prune stale weak references, then collect the remaining live handles.
    ///
    /// This method mutates the registry. Use [`Self::prune`] from a maintenance task
    /// to clean up without collecting handles. `DashMap::retain` holds
    /// per-bucket write locks while iterating.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     push::PushQueues,
    ///     session::{ConnectionId, SessionRegistry},
    /// };
    ///
    /// let registry = SessionRegistry::default();
    /// let (_q1, h1) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let (_q2, h2) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let id1 = ConnectionId::new(5);
    /// let id2 = ConnectionId::new(6);
    /// registry.insert(id1, &h1);
    /// registry.insert(id2, &h2);
    /// drop(h1);
    /// let active = registry.active_handles();
    /// assert_eq!(active.len(), 1);
    /// assert_eq!(active[0].0, id2);
    /// ```
    #[must_use]
    pub fn active_handles(&self) -> Vec<(ConnectionId, PushHandle<F>)> {
        self.retain_and_collect(|id, inner| (id, PushHandle::from_arc(inner)))
    }

    /// Prune stale weak references, then return the IDs of the live connections.
    ///
    /// This method mutates the registry. Use [`Self::prune`] from a maintenance task
    /// to clean up without collecting handles. `DashMap::retain` holds
    /// per-bucket write locks while iterating.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{
    ///     push::PushQueues,
    ///     session::{ConnectionId, SessionRegistry},
    /// };
    ///
    /// let registry = SessionRegistry::default();
    /// let (_q1, h1) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let (_q2, h2) = PushQueues::<u8>::builder()
    ///     .high_capacity(1)
    ///     .low_capacity(1)
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let id1 = ConnectionId::new(7);
    /// let id2 = ConnectionId::new(8);
    /// registry.insert(id1, &h1);
    /// registry.insert(id2, &h2);
    /// drop(h1);
    /// let ids = registry.active_ids();
    /// assert_eq!(ids, vec![id2]);
    /// ```
    #[must_use]
    pub fn active_ids(&self) -> Vec<ConnectionId> { self.retain_and_collect(|id, _| id) }
}

#[cfg(test)]
mod tests {
    //! Unit tests for lazy dead-entry cleanup in the session registry.

    use super::{ConnectionId, SessionRegistry};
    use crate::push::PushQueues;

    /// A failed lookup must also evict the dead weak entry; retaining it
    /// grows the registry until a later `prune`, which is the observable
    /// difference the `strong_count() == 0` guard protects.
    #[tokio::test]
    async fn get_evicts_dead_entry_from_registry() {
        let registry = SessionRegistry::<u8>::default();
        let id = ConnectionId::new(1);

        let (queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(1)
            .low_capacity(1)
            .build()
            .expect("failed to build PushQueues");
        registry.insert(id, &handle);
        drop(handle);
        drop(queues);

        assert!(registry.get(&id).is_none(), "dead handle must not upgrade");
        assert!(
            registry.entries.is_empty(),
            "failed lookup must evict the dead entry rather than leave it to prune"
        );
    }

    #[test]
    fn connection_ids_start_at_one_and_increase() {
        let registry = SessionRegistry::<u8>::new();

        assert_eq!(registry.next_connection_id(), ConnectionId::new(1));
        assert_eq!(registry.next_connection_id(), ConnectionId::new(2));
        assert_eq!(registry.next_connection_id(), ConnectionId::new(3));
    }

    #[test]
    fn default_registry_allocates_ids_from_one() {
        let registry = SessionRegistry::<u8>::default();

        assert_eq!(registry.next_connection_id(), ConnectionId::new(1));
    }
}
