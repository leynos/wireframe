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
    /// The first identifier allocated by [`Self::allocate_connection_id`] is `1`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::session::{ConnectionId, SessionRegistry};
    ///
    /// let registry = SessionRegistry::<u8>::new();
    /// assert_eq!(registry.allocate_connection_id(), ConnectionId::new(1));
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocate the next connection identifier no entry currently occupies.
    ///
    /// Identifiers are unique for the lifetime of the registry and are never
    /// reused. Callers should allocate one per accepted connection and store
    /// it alongside the connection's push handle.
    ///
    /// An identifier already present in the registry is skipped rather than
    /// handed out. An application that still inserts entries by hand therefore
    /// keeps them: the first server connection receives `1` only when no entry
    /// occupies it, and otherwise the next free identifier.
    ///
    /// # Panics
    ///
    /// Panics when the `u64` identifier space is exhausted. Reaching that
    /// point requires allocating every possible identifier, so the panic
    /// reports an unrecoverable configuration error rather than wrapping and
    /// handing out a duplicate.
    #[must_use]
    pub fn allocate_connection_id(&self) -> ConnectionId {
        loop {
            let updated =
                self.next_id
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                        current.checked_add(1)
                    });
            let Ok(id) = updated else {
                panic!("connection identifier space exhausted");
            };
            let candidate = ConnectionId::new(id);
            if !self.entries.contains_key(&candidate) {
                return candidate;
            }
        }
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
    /// assert!(registry.insert(id, &handle));
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
    /// Returns `true` when the handle was published, and `false` when a live
    /// connection already owns the identifier: its entry is kept, so a manual
    /// insert cannot displace the handle of a connection the registry
    /// allocated that identifier to. Call [`Self::remove`] first to replace a
    /// live entry deliberately.
    ///
    /// A dead entry does not block publication. Its weak reference cannot
    /// upgrade, so replacing it publishes the new handle under the same
    /// identifier without affecting any live connection.
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
    /// assert!(registry.insert(id, &handle));
    /// assert!(registry.get(&id).is_some());
    /// // A second publish would displace a live connection, so it is refused.
    /// assert!(!registry.insert(id, &handle));
    /// ```
    #[must_use]
    pub fn insert(&self, id: ConnectionId, handle: &PushHandle<F>) -> bool {
        match self.entries.entry(id) {
            dashmap::mapref::entry::Entry::Occupied(mut occupied) => {
                if occupied.get().strong_count() > 0 {
                    return false;
                }
                occupied.insert(handle.downgrade());
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                vacant.insert(handle.downgrade());
            }
        }
        true
    }

    /// Allocate an identifier and publish `handle` under it in one step.
    ///
    /// Allocation and publication are a single claim, so a manual
    /// [`Self::insert`] racing this call cannot leave the connection addressing
    /// a handle it does not own.
    ///
    /// # Panics
    ///
    /// Panics when the `u64` identifier space is exhausted.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::{push::PushQueues, session::SessionRegistry};
    /// let (_queues, handle) = PushQueues::<u8>::builder()
    ///     .unlimited()
    ///     .build()
    ///     .expect("failed to build PushQueues");
    /// let registry = SessionRegistry::default();
    /// let id = registry.allocate_and_publish(&handle);
    /// assert!(registry.get(&id).is_some());
    /// ```
    #[must_use]
    pub fn allocate_and_publish(&self, handle: &PushHandle<F>) -> ConnectionId {
        loop {
            let candidate = self.allocate_connection_id();
            if self.insert(candidate, handle) {
                return candidate;
            }
        }
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
    /// assert!(registry.insert(id, &handle));
    /// assert!(registry.get(&id).is_some());
    /// registry.remove(&id);
    /// assert!(registry.get(&id).is_none());
    /// ```
    pub fn remove(&self, id: &ConnectionId) { self.entries.remove(id); }

    /// Remove the entry for `id` only while it still holds `handle`, reporting
    /// whether one was removed. Teardown uses this rather than [`Self::remove`],
    /// so a connection cannot remove a handle a later insert published under its
    /// old identifier.
    pub(crate) fn remove_owned(&self, id: &ConnectionId, handle: &PushHandle<F>) -> bool {
        let owned = handle.downgrade();
        self.entries
            .remove_if(id, |_, weak| Weak::ptr_eq(weak, &owned))
            .is_some()
    }

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
    /// assert!(registry.insert(id, &handle));
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
    /// assert!(registry.insert(id1, &h1));
    /// assert!(registry.insert(id2, &h2));
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
    /// assert!(registry.insert(id1, &h1));
    /// assert!(registry.insert(id2, &h2));
    /// drop(h1);
    /// let ids = registry.active_ids();
    /// assert_eq!(ids, vec![id2]);
    /// ```
    #[must_use]
    pub fn active_ids(&self) -> Vec<ConnectionId> { self.retain_and_collect(|id, _| id) }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
